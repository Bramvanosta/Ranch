//! What needs the user across every connection:
//! failure causes, stuck queues (paused longer than a
//! threshold, or jobs waiting with no worker) and connections that cannot
//! be reached. Composed from the per-connection reads that already exist;
//! the connections are read side by side, each bounded in time, so one dead
//! server holds nobody up.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::SqlitePool;

#[cfg(feature = "desktop")]
use crate::app_state::AppState;
use crate::commands::audit::{list_audit_log_entries_inner, ListAuditLogEntriesArgs};
use crate::commands::connections::{list_connections_inner, resolve_connection_config, Connection};
use crate::commands::jobs::ListFailureCausesArgs;
use crate::commands::queues::CountQueueWorkersArgs;
use crate::core::audit::{AuditOutcome, AuditTarget};
use crate::core::bullmq_schema::{JobRef, Queue};
use crate::core::connection_actor::ConnectionRegistry;
use crate::core::errors::RanchError;
#[cfg(feature = "desktop")]
use crate::core::errors::RanchErrorDto;
use crate::core::keychain::KeychainAdapter;

/// A connection that does not answer in this long counts as offline.
const PER_CONNECTION_TIMEOUT: Duration = Duration::from_secs(6);
/// Failure causes listed per connection; the Failed tab has the rest.
const CAUSES_PER_CONNECTION: usize = 10;
/// Paused longer than this = stuck (default).
const DEFAULT_PAUSED_THRESHOLD_MS: f64 = 15.0 * 60.0 * 1000.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AttentionKind {
    Failing,
    Stuck,
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AttentionItem {
    /// Stable across reads — what a mute remembers.
    pub key: String,
    pub kind: AttentionKind,
    pub connection_id: String,
    pub connection_name: String,
    pub env_color: String,
    pub env_label: String,
    /// The queue, or the one queue of a cause; `None` for a connection.
    pub queue_name: Option<String>,
    /// Failing: the cause's signature. Stuck: `paused` or `noWorker`.
    /// Offline: what Redis (or the network) said.
    pub reason: String,
    /// Failing: the newest reason as written.
    pub sample: Option<String>,
    /// Failing: failed jobs of the cause. Stuck: jobs waiting.
    pub count: u32,
    /// Since when (ms): first failure of the cause, the pause (from the
    /// audit log), `None` when not known.
    pub since_ms: Option<f64>,
    /// Failing: failures per hour over the last 24 hours.
    pub trend: Vec<u32>,
    /// Failing: the cause's jobs, for "Retry N".
    pub jobs: Vec<JobRef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AttentionReport {
    /// Most urgent first.
    pub items: Vec<AttentionItem>,
    pub connections_checked: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListAttentionArgs {
    /// Paused longer than this counts as stuck (default 15 min).
    #[serde(default)]
    pub paused_threshold_ms: Option<f64>,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_attention(
    args: ListAttentionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<AttentionReport, RanchErrorDto> {
    list_attention_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn list_attention_inner(
    args: ListAttentionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<AttentionReport, RanchError> {
    let connections = list_connections_inner(pool).await?;
    let now_ms = chrono::Utc::now().timestamp_millis() as f64;
    let threshold = args
        .paused_threshold_ms
        .unwrap_or(DEFAULT_PAUSED_THRESHOLD_MS);
    let pauses = latest_pauses(pool).await;
    let reads = join_all(
        connections
            .iter()
            .map(|c| read_connection(c, pool, keychain, registry))
            .collect(),
    )
    .await;
    let mut items = Vec::new();
    for (c, read) in connections.iter().zip(reads) {
        items.extend(items_of(c, read, &pauses, now_ms, threshold));
    }
    Ok(AttentionReport {
        items: sort_by_urgency(items),
        connections_checked: connections.len() as u32,
    })
}

/// What one connection told: its queues, worker counts and causes — or why
/// it could not be read.
struct ConnectionRead {
    queues: Vec<Queue>,
    workers: Option<std::collections::HashMap<String, u32>>,
    causes: Vec<crate::core::bullmq_schema::FailureCause>,
}

async fn read_connection(
    c: &Connection,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<ConnectionRead, String> {
    let work = async {
        let config = resolve_connection_config(&c.id, pool, keychain)
            .await
            .map_err(|e| e.to_string())?;
        let queues = match registry.list_queues(&c.id, &config).await {
            Ok(Ok(q)) => q,
            Ok(Err(e)) => return Err(e.to_string()),
            Err(e) => return Err(e.to_string()),
        };
        let workers = match registry
            .count_queue_workers(
                &c.id,
                &config,
                CountQueueWorkersArgs {
                    connection_id: c.id.clone(),
                    queue_refs: queues.iter().map(|q| q.queue_ref.clone()).collect(),
                },
            )
            .await
        {
            Ok(Ok(w)) if w.supported => Some(
                w.counts
                    .into_iter()
                    .map(|c| (c.queue_ref.name, c.workers))
                    .collect(),
            ),
            _ => None,
        };
        let causes = match registry
            .list_failure_causes(
                &c.id,
                &config,
                ListFailureCausesArgs {
                    connection_id: c.id.clone(),
                    queue: None,
                },
            )
            .await
        {
            Ok(Ok(report)) => report.causes,
            _ => Vec::new(),
        };
        Ok(ConnectionRead {
            queues,
            workers,
            causes,
        })
    };
    match tokio::time::timeout(PER_CONNECTION_TIMEOUT, work).await {
        Ok(read) => read,
        Err(_) => Err("No answer within 6 seconds".into()),
    }
}

/// When each queue was last paused from Ranch: `(connection, queue)` → ms.
async fn latest_pauses(pool: &SqlitePool) -> std::collections::HashMap<(String, String), f64> {
    let mut out = std::collections::HashMap::new();
    let Ok(entries) = list_audit_log_entries_inner(
        ListAuditLogEntriesArgs {
            from: None,
            to: None,
            connection_id: None,
            action: Some("pauseQueue".into()),
            source: None,
            offset: 0,
            limit: 500,
        },
        pool,
    )
    .await
    else {
        return out;
    };
    // Newest first: the first entry per queue is the pause that holds.
    for e in entries {
        let (Some(connection), AuditTarget::Queue { queue_name }) = (e.connection_id, e.target)
        else {
            continue;
        };
        if e.outcome != AuditOutcome::Success {
            continue;
        }
        if let Ok(at) = chrono::DateTime::parse_from_rfc3339(&e.created_at) {
            out.entry((connection, queue_name))
                .or_insert(at.timestamp_millis() as f64);
        }
    }
    out
}

fn items_of(
    c: &Connection,
    read: Result<ConnectionRead, String>,
    pauses: &std::collections::HashMap<(String, String), f64>,
    now_ms: f64,
    paused_threshold_ms: f64,
) -> Vec<AttentionItem> {
    let base = |kind: AttentionKind, key: String| AttentionItem {
        key,
        kind,
        connection_id: c.id.clone(),
        connection_name: c.name.clone(),
        env_color: c.env_color.clone(),
        env_label: c.env_label.clone(),
        queue_name: None,
        reason: String::new(),
        sample: None,
        count: 0,
        since_ms: None,
        trend: Vec::new(),
        jobs: Vec::new(),
    };
    let read = match read {
        Ok(r) => r,
        Err(message) => {
            return vec![AttentionItem {
                reason: message,
                ..base(AttentionKind::Offline, format!("offline:{}", c.id))
            }]
        }
    };
    let mut items = Vec::new();
    for cause in read.causes.into_iter().take(CAUSES_PER_CONNECTION) {
        items.push(AttentionItem {
            queue_name: (cause.queue_names.len() == 1).then(|| cause.queue_names[0].clone()),
            reason: cause.signature.clone(),
            sample: Some(cause.sample.clone()),
            count: cause.count,
            since_ms: Some(cause.first_at as f64),
            trend: cause.trend.clone(),
            jobs: cause.jobs.clone(),
            ..base(
                AttentionKind::Failing,
                format!("failing:{}:{}", c.id, cause.signature),
            )
        });
    }
    for q in read.queues {
        let name = q.queue_ref.name.clone();
        let waiting = (q.counters.waiting + q.counters.prioritized + q.counters.paused)
            .min(u32::MAX as u64) as u32;
        if q.is_paused {
            let since = pauses.get(&(c.id.clone(), name.clone())).copied();
            // Unknown since when (paused outside Ranch): stuck all the same.
            if since.is_none_or(|t| now_ms - t >= paused_threshold_ms) {
                items.push(AttentionItem {
                    queue_name: Some(name.clone()),
                    reason: "paused".into(),
                    count: waiting,
                    since_ms: since,
                    ..base(AttentionKind::Stuck, format!("paused:{}:{name}", c.id))
                });
            }
        } else if waiting > 0
            && read
                .workers
                .as_ref()
                .is_some_and(|w| w.get(&name).copied() == Some(0))
        {
            items.push(AttentionItem {
                queue_name: Some(name.clone()),
                reason: "noWorker".into(),
                count: waiting,
                ..base(AttentionKind::Stuck, format!("noworker:{}:{name}", c.id))
            });
        }
    }
    items
}

/// Production before staging before the rest; within an environment the
/// offline connections, then failing (most failures first), then stuck
/// (most jobs waiting first).
pub fn sort_by_urgency(mut items: Vec<AttentionItem>) -> Vec<AttentionItem> {
    let env = |c: &str| match c {
        "env-production" => 0,
        "env-staging" => 1,
        _ => 2,
    };
    let kind = |k: AttentionKind| match k {
        AttentionKind::Offline => 0,
        AttentionKind::Failing => 1,
        AttentionKind::Stuck => 2,
    };
    items.sort_by(|a, b| {
        env(&a.env_color)
            .cmp(&env(&b.env_color))
            .then(kind(a.kind).cmp(&kind(b.kind)))
            .then(b.count.cmp(&a.count))
            .then(a.key.cmp(&b.key))
    });
    items
}

/// The futures side by side, results in order (no `futures` crate here).
async fn join_all<F: Future>(futures: Vec<F>) -> Vec<F::Output> {
    let mut pending: Vec<Option<Pin<Box<F>>>> =
        futures.into_iter().map(|f| Some(Box::pin(f))).collect();
    let mut results: Vec<Option<F::Output>> = (0..pending.len()).map(|_| None).collect();
    std::future::poll_fn(|cx: &mut Context<'_>| {
        let mut done = true;
        for (slot, result) in pending.iter_mut().zip(results.iter_mut()) {
            if let Some(fut) = slot {
                match fut.as_mut().poll(cx) {
                    Poll::Ready(out) => {
                        *result = Some(out);
                        *slot = None;
                    }
                    Poll::Pending => done = false,
                }
            }
        }
        if done {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
    results.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: AttentionKind, env: &str, count: u32, key: &str) -> AttentionItem {
        AttentionItem {
            key: key.into(),
            kind,
            connection_id: "c".into(),
            connection_name: "c".into(),
            env_color: env.into(),
            env_label: env.into(),
            queue_name: None,
            reason: String::new(),
            sample: None,
            count,
            since_ms: None,
            trend: Vec::new(),
            jobs: Vec::new(),
        }
    }

    #[test]
    fn most_urgent_first() {
        use AttentionKind::*;
        let sorted = sort_by_urgency(vec![
            item(Stuck, "env-local", 50, "a"),
            item(Failing, "env-staging", 3, "b"),
            item(Failing, "env-production", 2, "c"),
            item(Stuck, "env-production", 9, "d"),
            item(Failing, "env-production", 40, "e"),
            item(Offline, "env-production", 0, "f"),
        ]);
        assert_eq!(
            sorted.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(),
            vec!["f", "e", "c", "d", "b", "a"]
        );
    }

    #[tokio::test]
    async fn join_all_keeps_the_order() {
        let out = join_all(vec![
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                1
            }) as Pin<Box<dyn Future<Output = i32>>>,
            Box::pin(async { 2 }),
        ])
        .await;
        assert_eq!(out, vec![1, 2]);
    }
}
