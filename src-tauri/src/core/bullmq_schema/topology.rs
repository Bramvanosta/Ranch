//! What the Map tab needs beyond counters, throughput and
//! workers: which queues feed which through flows, how fast jobs are
//! added, and how old the oldest waiting job is.
//!
//! - **Flow links**: a parent waiting for its children lists them in
//!   `<parent>:dependencies`; a finished parent keeps what they returned in
//!   `<parent>:processed`. Both hold child keys (`<prefix>:<queue>:<id>`), so
//!   the newest parents of each queue tell which queues it hands work to.
//! - **Added per minute**: the `added` events of the last hour in the queue's
//!   event stream (capped: a busy queue's stream is trimmed).

use serde::{Deserialize, Serialize};
use specta::Type;

use super::domain::QueueRef;

/// Parents read per queue and source (waiting-children, completed).
pub const PARENTS_PER_QUEUE: usize = 50;
/// Events read per queue for the added rate.
pub const EVENTS_PER_QUEUE: usize = 2_000;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowLink {
    pub parent: QueueRef,
    pub child: QueueRef,
    /// Child jobs seen on the link (among the newest parents).
    pub jobs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueFlowStats {
    pub queue_ref: QueueRef,
    /// `added` events per minute over the last hour.
    pub added_per_minute: f64,
    /// The stream did not reach back an hour: a lower bound.
    pub added_capped: bool,
    /// When the oldest waiting job was added (ms); `None` = nothing waits.
    pub oldest_waiting_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Topology {
    pub links: Vec<FlowLink>,
    pub queues: Vec<QueueFlowStats>,
}

/// `bull:emails:42` → `("bull", "emails")` when `prefix` is `bull`.
/// The id is the last segment; the queue is what sits between.
pub fn queue_of_job_key(prefix: &str, key: &str) -> Option<QueueRef> {
    let rest = key.strip_prefix(prefix)?.strip_prefix(':')?;
    let (queue, id) = rest.rsplit_once(':')?;
    (!queue.is_empty() && !id.is_empty()).then(|| QueueRef {
        prefix: prefix.to_string(),
        name: queue.to_string(),
    })
}

/// Count parent → child queue pairs.
pub fn links(pairs: impl IntoIterator<Item = (QueueRef, QueueRef)>) -> Vec<FlowLink> {
    let mut out: Vec<FlowLink> = Vec::new();
    for (parent, child) in pairs {
        match out
            .iter_mut()
            .find(|l| l.parent == parent && l.child == child)
        {
            Some(link) => link.jobs += 1,
            None => out.push(FlowLink {
                parent,
                child,
                jobs: 1,
            }),
        }
    }
    out.sort_by(|a, b| b.jobs.cmp(&a.jobs).then(a.parent.name.cmp(&b.parent.name)));
    out
}

/// `added` events per minute in the last hour, from stream entries
/// `(id, event name)` newest first; `capped` when the stream ends inside
/// the hour while full.
pub fn added_rate(entries: &[(String, String)], now_ms: i64, read_cap: usize) -> (f64, bool) {
    let since = now_ms - 3_600_000;
    let ts = |id: &str| id.split('-').next().and_then(|t| t.parse::<i64>().ok());
    let added = entries
        .iter()
        .filter(|(id, event)| event == "added" && ts(id).is_some_and(|t| t >= since))
        .count();
    let oldest = entries.last().and_then(|(id, _)| ts(id));
    let capped = entries.len() >= read_cap && oldest.is_some_and(|t| t > since);
    (added as f64 / 60.0, capped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(name: &str) -> QueueRef {
        QueueRef {
            prefix: "bull".into(),
            name: name.into(),
        }
    }

    #[test]
    fn a_child_key_names_its_queue() {
        assert_eq!(
            queue_of_job_key("bull", "bull:emails:42"),
            Some(q("emails"))
        );
        assert_eq!(
            queue_of_job_key("bull", "bull:eu:webhooks:7"),
            Some(q("eu:webhooks")),
            "a queue name may hold colons; the id is the last segment"
        );
        assert_eq!(queue_of_job_key("bull", "other:emails:42"), None);
        assert_eq!(queue_of_job_key("bull", "bull:emails"), None);
    }

    #[test]
    fn links_are_counted_and_sorted() {
        let l = links([
            (q("orders"), q("emails")),
            (q("orders"), q("emails")),
            (q("orders"), q("invoices")),
        ]);
        assert_eq!(l[0].child, q("emails"));
        assert_eq!(l[0].jobs, 2);
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn the_added_rate_counts_the_last_hour() {
        let now = 10_000_000;
        let e = |t: i64, ev: &str| (format!("{t}-0"), ev.to_string());
        let entries = vec![
            e(now - 1_000, "added"),
            e(now - 2_000, "completed"),
            e(now - 60_000, "added"),
            e(now - 4_000_000, "added"),
        ];
        let (rate, capped) = added_rate(&entries, now, 2_000);
        assert!((rate - 2.0 / 60.0).abs() < 1e-9);
        assert!(!capped);
        let (_, capped) = added_rate(&entries[..2], now, 2);
        assert!(capped, "full read, still inside the hour");
    }
}
