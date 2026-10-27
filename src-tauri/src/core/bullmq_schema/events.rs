//! Live events of a connection.
//!
//! BullMQ appends what happens to each queue's `…:events` stream (`added`,
//! `active`, `completed`, `failed`…). The overview reads them while it is on
//! screen: from a cursor per queue, newest first, capped — above the cap the
//! newest are kept and the reply says it sampled. Pure: the actor reads the
//! streams, this decides what to show.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Per queue and read: more than this since the cursor = sampled.
pub const PER_QUEUE_CAP: usize = 50;
/// The first read of a queue goes back this far.
pub const FIRST_READ: usize = 20;

/// Where the last read of a queue stopped (a stream id).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EventCursor {
    pub queue_name: String,
    pub last_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueEvent {
    pub queue_name: String,
    /// The stream id (`<ms>-<seq>`), unique within the queue.
    pub stream_id: String,
    pub at_ms: f64,
    /// `added`, `active`, `completed`, `failed`, `delayed`, `stalled`,
    /// `removed`, or `retried` (BullMQ's `waiting` after a failure).
    pub event: String,
    pub job_id: Option<String>,
    pub job_name: Option<String>,
    pub failed_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueEvents {
    /// Newest first.
    pub events: Vec<QueueEvent>,
    /// Where to read from next time, one per queue read.
    pub cursors: Vec<EventCursor>,
    /// More happened than was read: the newest were kept.
    pub sampled: bool,
}

/// `<ms>-<seq>` as a pair, for ordering; an odd id sorts first.
pub fn parse_stream_id(id: &str) -> (u64, u64) {
    let (ms, seq) = id.split_once('-').unwrap_or((id, "0"));
    (ms.parse().unwrap_or(0), seq.parse().unwrap_or(0))
}

/// One stream entry, as `XREVRANGE` returns it, into an event to show —
/// `None` for the kinds the panel does not list (`progress`, `drained`…).
pub fn to_event(queue_name: &str, id: &str, fields: &[String]) -> Option<QueueEvent> {
    let get = |name: &str| -> Option<String> {
        fields
            .chunks(2)
            .find(|kv| kv.first().map(String::as_str) == Some(name))
            .and_then(|kv| kv.get(1).cloned())
    };
    let raw = get("event")?;
    let event = match raw.as_str() {
        "added" | "active" | "completed" | "failed" | "delayed" | "stalled" | "removed" => raw,
        "waiting" if get("prev").as_deref() == Some("failed") => "retried".to_string(),
        _ => return None,
    };
    Some(QueueEvent {
        queue_name: queue_name.to_string(),
        stream_id: id.to_string(),
        at_ms: parse_stream_id(id).0 as f64,
        event,
        job_id: get("jobId"),
        job_name: get("name"),
        failed_reason: get("failedReason"),
    })
}

/// One `XREVRANGE` entry: its stream id and its flat field / value list.
pub type StreamEntry = (String, Vec<String>);

/// What one queue's read brought: its entries newer than the cursor (the
/// read is inclusive of it), whether it overflowed, and the next cursor.
pub fn after_cursor(
    entries: Vec<StreamEntry>,
    cursor: Option<&str>,
    asked: usize,
) -> (Vec<StreamEntry>, bool, Option<String>) {
    let newest = entries.first().map(|(id, _)| id.clone());
    let Some(cursor) = cursor else {
        return (entries, false, newest);
    };
    let floor = parse_stream_id(cursor);
    let reached_cursor = entries.iter().any(|(id, _)| parse_stream_id(id) <= floor);
    let fresh: Vec<_> = entries
        .into_iter()
        .filter(|(id, _)| parse_stream_id(id) > floor)
        .collect();
    let overflowed = !reached_cursor && fresh.len() >= asked;
    let next = newest.or_else(|| Some(cursor.to_string()));
    (fresh, overflowed, next)
}

/// Merge the queues' events, newest first, at most `limit`.
pub fn merge(mut events: Vec<QueueEvent>, limit: usize) -> (Vec<QueueEvent>, bool) {
    events.sort_by(|a, b| {
        parse_stream_id(&b.stream_id)
            .cmp(&parse_stream_id(&a.stream_id))
            .then_with(|| a.queue_name.cmp(&b.queue_name))
    });
    let cut = events.len() > limit;
    events.truncate(limit);
    (events, cut)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, kv: &[(&str, &str)]) -> StreamEntry {
        (
            id.into(),
            kv.iter()
                .flat_map(|(k, v)| [k.to_string(), v.to_string()])
                .collect(),
        )
    }

    #[test]
    fn keeps_the_kinds_the_panel_lists() {
        let (id, f) = entry(
            "1700000000123-0",
            &[
                ("event", "failed"),
                ("jobId", "42"),
                ("failedReason", "boom"),
                ("prev", "active"),
            ],
        );
        let e = to_event("emails", &id, &f).unwrap();
        assert_eq!(e.event, "failed");
        assert_eq!(e.job_id.as_deref(), Some("42"));
        assert_eq!(e.failed_reason.as_deref(), Some("boom"));
        assert_eq!(e.at_ms, 1_700_000_000_123.0);

        let (id, f) = entry(
            "1-0",
            &[("event", "waiting"), ("jobId", "42"), ("prev", "failed")],
        );
        assert_eq!(to_event("q", &id, &f).unwrap().event, "retried");
        let (id, f) = entry("1-0", &[("event", "waiting"), ("jobId", "42")]);
        assert!(
            to_event("q", &id, &f).is_none(),
            "a plain waiting repeats added"
        );
        let (id, f) = entry(
            "1-0",
            &[("event", "progress"), ("jobId", "42"), ("data", "50")],
        );
        assert!(to_event("q", &id, &f).is_none());
        let (id, f) = entry(
            "1-0",
            &[("event", "added"), ("jobId", "7"), ("name", "send")],
        );
        assert_eq!(
            to_event("q", &id, &f).unwrap().job_name.as_deref(),
            Some("send")
        );
    }

    #[test]
    fn a_read_after_a_cursor_drops_the_cursor_and_says_when_it_overflowed() {
        // XREVRANGE + cursor COUNT 3: newest first, the cursor included when reached.
        let reached = vec![entry("30-0", &[]), entry("20-0", &[]), entry("10-0", &[])];
        let (fresh, overflowed, next) = after_cursor(reached, Some("10-0"), 3);
        assert_eq!(fresh.len(), 2);
        assert!(!overflowed);
        assert_eq!(next.as_deref(), Some("30-0"));

        let full = vec![entry("90-0", &[]), entry("80-0", &[]), entry("70-0", &[])];
        let (fresh, overflowed, _) = after_cursor(full, Some("10-0"), 3);
        assert_eq!(fresh.len(), 3);
        assert!(
            overflowed,
            "the cursor was not reached: older events were skipped"
        );

        let (fresh, overflowed, next) = after_cursor(Vec::new(), Some("10-0"), 3);
        assert!(fresh.is_empty() && !overflowed);
        assert_eq!(
            next.as_deref(),
            Some("10-0"),
            "nothing new: the cursor stays"
        );

        let (fresh, _, next) = after_cursor(vec![entry("5-1", &[])], None, 20);
        assert_eq!(fresh.len(), 1);
        assert_eq!(next.as_deref(), Some("5-1"));
    }

    #[test]
    fn merged_newest_first_across_queues_and_capped() {
        let ev = |q: &str, id: &str| QueueEvent {
            queue_name: q.into(),
            stream_id: id.into(),
            at_ms: 0.0,
            event: "completed".into(),
            job_id: None,
            job_name: None,
            failed_reason: None,
        };
        let (out, cut) = merge(
            vec![
                ev("a", "10-0"),
                ev("b", "30-0"),
                ev("a", "20-1"),
                ev("b", "20-0"),
            ],
            3,
        );
        assert_eq!(
            out.iter().map(|e| e.stream_id.as_str()).collect::<Vec<_>>(),
            vec!["30-0", "20-1", "20-0"]
        );
        assert!(cut);
    }
}
