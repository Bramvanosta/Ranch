//! Queue maintenance: drain, clean and retry-all-failed, run as batched
//! Lua scripts with progress events, and the read-only preview of how
//! many jobs one would touch.

use tokio_util::sync::CancellationToken;

use super::authorize::{authorize_write, write_audit_for_action};
use crate::bullmq_schema::maintenance::{self, BatchReply};
use crate::bullmq_schema::{
    CleanState, KeyVariant, Keys, MaintenancePreview, MaintenanceResult, QueueMaintenance, QueueRef,
};
use crate::connection_actor::actor::ActorState;
use crate::connection_actor::link::{
    open_redis_for_actor, redis_to_ranch_err, resolve_keys_for_queue, RedisConn,
};
use ranch_application::ports::ProgressSink;
use ranch_domain::audit::{AuditAction, AuditOutcome, AuditTarget, Caller};
use ranch_domain::errors::RanchError;
use ranch_domain::jobs::BulkProgress;
use ranch_domain::queues::QueueId;

/// Ids of the pending runs of the queue's job schedulers
/// (`repeat:{scheduler}:{next millis}`), as `drain_queue.lua` computes them.
async fn scheduler_job_ids(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
) -> Result<Vec<String>, redis::RedisError> {
    let schedulers: Vec<(String, f64)> = redis::cmd("ZRANGE")
        .arg(keys.repeat(qref))
        .arg(0)
        .arg(-1)
        .arg("WITHSCORES")
        .query_async(conn)
        .await?;
    Ok(schedulers
        .into_iter()
        .map(|(id, next)| format!("repeat:{id}:{}", next as i64))
        .collect())
}

/// What a maintenance operation would touch. Exact for
/// clean and retry; for drain an upper bound minus the scheduler runs it
/// spares (flow members are only recognised while running).
pub(crate) async fn handle_preview_queue_maintenance(
    state: &mut ActorState,
    queue_id: &QueueId,
    operation: &QueueMaintenance,
    token: &CancellationToken,
) -> Result<MaintenancePreview, RanchError> {
    let mut conn = open_redis_for_actor(state, token).await?;
    let q = &QueueRef::parse(queue_id.as_str())?;
    let keys = resolve_keys_for_queue(state, &mut conn, q, token).await?;
    let now = chrono::Utc::now().timestamp_millis();

    let preview = async {
        match operation {
            QueueMaintenance::Drain { include_delayed } => {
                let (wait, paused, prioritized, delayed): (u64, u64, u64, u64) = redis::pipe()
                    .cmd("LLEN")
                    .arg(keys.wait(q))
                    .cmd("LLEN")
                    .arg(keys.paused(q))
                    .cmd("ZCARD")
                    .arg(keys.prioritized(q))
                    .cmd("ZCARD")
                    .arg(keys.delayed(q))
                    .query_async(&mut conn)
                    .await?;
                // A scheduler's pending run sits in `delayed`, or in `wait`
                // when it is due now (an `every` scheduler's first run is).
                let mut spared = 0u64;
                for id in scheduler_job_ids(&mut conn, keys, q).await? {
                    let (in_wait, in_paused, in_prioritized, in_delayed): (
                        Option<i64>,
                        Option<i64>,
                        Option<f64>,
                        Option<f64>,
                    ) = redis::pipe()
                        .cmd("LPOS")
                        .arg(keys.wait(q))
                        .arg(&id)
                        .cmd("LPOS")
                        .arg(keys.paused(q))
                        .arg(&id)
                        .cmd("ZSCORE")
                        .arg(keys.prioritized(q))
                        .arg(&id)
                        .cmd("ZSCORE")
                        .arg(keys.delayed(q))
                        .arg(&id)
                        .query_async(&mut conn)
                        .await?;
                    if in_wait.is_some()
                        || in_paused.is_some()
                        || in_prioritized.is_some()
                        || (*include_delayed && in_delayed.is_some())
                    {
                        spared += 1;
                    }
                }
                let delayed = if *include_delayed { delayed } else { 0 };
                Ok::<_, redis::RedisError>(MaintenancePreview {
                    affected: (wait + paused + prioritized + delayed).saturating_sub(spared),
                    spared_scheduler_jobs: spared,
                })
            }
            QueueMaintenance::Clean {
                state: clean_state,
                older_than_ms,
            } => {
                let set = match clean_state {
                    CleanState::Completed => keys.completed(q),
                    CleanState::Failed => keys.failed(q),
                };
                let cutoff = now - i64::try_from(*older_than_ms).unwrap_or(i64::MAX);
                let affected: u64 = redis::cmd("ZCOUNT")
                    .arg(set)
                    .arg("-inf")
                    .arg(cutoff)
                    .query_async(&mut conn)
                    .await?;
                Ok(MaintenancePreview {
                    affected,
                    spared_scheduler_jobs: 0,
                })
            }
            QueueMaintenance::RetryAllFailed => {
                let affected: u64 = redis::cmd("ZCARD")
                    .arg(keys.failed(q))
                    .query_async(&mut conn)
                    .await?;
                Ok(MaintenancePreview {
                    affected,
                    spared_scheduler_jobs: 0,
                })
            }
        }
    };
    tokio::select! {
        r = preview => r.map_err(redis_to_ranch_err),
        _ = token.cancelled() => Err(RanchError::Cancelled),
    }
}

/// Drain / clean / retry all failed.
///
///   1. safety gate, once, for the whole operation;
///   2. the preview count becomes the progress total;
///   3. the operation's script runs in batches of `maintenance::BATCH`;
///      between two batches: cancellation check + a `bulk:progress` event
///      (the same event the bulk actions emit, so the same progress UI);
///   4. one audit row: what was done, what was left in place, whether it
///      was interrupted, also when a batch failed half-way.
pub(crate) async fn handle_run_queue_maintenance(
    state: &mut ActorState,
    queue_id: &QueueId,
    op: &QueueMaintenance,
    caller: &Caller,
    sink: &dyn ProgressSink,
    token: &CancellationToken,
) -> Result<MaintenanceResult, RanchError> {
    authorize_write(state, caller, op.tool()).await?;

    let total = handle_preview_queue_maintenance(state, queue_id, op, token)
        .await?
        .affected;
    let mut conn = open_redis_for_actor(state, token).await?;
    let q = &QueueRef::parse(queue_id.as_str())?;
    let keys = resolve_keys_for_queue(state, &mut conn, q, token).await?;
    let prefix = format!("{}:", keys.queue_base(q));
    let batch = maintenance::BATCH;
    let now = chrono::Utc::now().timestamp_millis();

    let mut affected: u64 = 0;
    let mut skipped: u64 = 0;
    let mut cancelled = false;
    let mut error: Option<redis::RedisError> = None;

    let progress = |affected: u64, skipped: u64| {
        let clamp = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
        sink.bulk(BulkProgress {
            action: op.verb().to_string(),
            queue_id: q.id(),
            total: clamp(total.max(affected + skipped)),
            completed: clamp(affected + skipped),
            failed_so_far: 0,
        });
    };

    // One container at a time; each loop iteration is one script call.
    // `offset` = jobs skipped so far, which stay at the head of the container.
    let containers: Vec<(String, &str)> = match op {
        QueueMaintenance::Drain { include_delayed } => {
            let mut c = vec![
                (keys.wait(q), "list"),
                (keys.paused(q), "list"),
                (keys.prioritized(q), "zset"),
            ];
            if *include_delayed {
                c.push((keys.delayed(q), "zset"));
            }
            c
        }
        QueueMaintenance::Clean {
            state: clean_state, ..
        } => vec![(
            match clean_state {
                CleanState::Completed => keys.completed(q),
                CleanState::Failed => keys.failed(q),
            },
            "zset",
        )],
        QueueMaintenance::RetryAllFailed => vec![(keys.failed(q), "zset")],
    };

    'containers: for (container, shape) in &containers {
        let mut offset: u64 = 0;
        loop {
            if token.is_cancelled() {
                cancelled = true;
                break 'containers;
            }
            let reply: Result<(i64, i64, i64), redis::RedisError> = match op {
                QueueMaintenance::Drain { .. } => {
                    redis::Script::new(&maintenance::drain_queue_lua())
                        .key(container)
                        .key(keys.repeat(q))
                        .arg(&prefix)
                        .arg(*shape)
                        .arg(offset)
                        .arg(batch)
                        .invoke_async(&mut conn)
                        .await
                }
                QueueMaintenance::Clean { older_than_ms, .. } => {
                    let cutoff = now - i64::try_from(*older_than_ms).unwrap_or(i64::MAX);
                    redis::Script::new(&maintenance::clean_jobs_lua())
                        .key(container)
                        .key(keys.events(q))
                        .arg(&prefix)
                        .arg(cutoff)
                        .arg(offset)
                        .arg(batch)
                        .invoke_async(&mut conn)
                        .await
                }
                QueueMaintenance::RetryAllFailed => {
                    redis::Script::new(&maintenance::retry_all_failed_lua())
                        .key(container)
                        .key(keys.wait(q))
                        .key(keys.paused(q))
                        .key(keys.meta(q))
                        .key(keys.active(q))
                        .key(keys.events(q))
                        .key(keys.marker(q))
                        .arg(&prefix)
                        .arg(now)
                        .arg(batch)
                        .invoke_async(&mut conn)
                        .await
                }
            };
            let reply = match reply {
                Ok(r) => BatchReply::from(r),
                Err(e) => {
                    error = Some(e);
                    break 'containers;
                }
            };
            let done = match op {
                // { retried, gone, remaining }
                QueueMaintenance::RetryAllFailed => {
                    affected += reply.first;
                    skipped += reply.second;
                    reply.third == 0 || reply.first + reply.second == 0
                }
                // { removed, skipped, scanned }
                _ => {
                    affected += reply.first;
                    skipped += reply.second;
                    offset += reply.second;
                    reply.third < batch as u64
                }
            };
            progress(affected, skipped);
            if done {
                break;
            }
        }
    }

    let (action, summary_verb) = match op {
        QueueMaintenance::Drain { .. } => (AuditAction::DrainQueue, "removed"),
        QueueMaintenance::Clean { .. } => (AuditAction::CleanQueue, "removed"),
        QueueMaintenance::RetryAllFailed => (AuditAction::RetryAllFailed, "retried"),
    };
    let mut reason = format!("{affected} {summary_verb}");
    if skipped > 0 {
        reason.push_str(&format!(", {skipped} left in place"));
    }
    if let QueueMaintenance::Clean {
        state: clean_state,
        older_than_ms,
    } = op
    {
        reason.push_str(&format!(
            " ({} older than {older_than_ms} ms)",
            match clean_state {
                CleanState::Completed => "completed",
                CleanState::Failed => "failed",
            }
        ));
    }
    let outcome = if let Some(e) = &error {
        reason.push_str(&format!("; stopped on error: {e}"));
        AuditOutcome::Failure
    } else if cancelled {
        reason.push_str("; cancelled");
        AuditOutcome::Cancelled
    } else {
        AuditOutcome::Success
    };
    write_audit_for_action(
        state,
        action,
        AuditTarget::queue(&q.id(), &q.name),
        outcome,
        Some(reason),
        caller,
    )
    .await?;

    if let Some(e) = error {
        return Err(redis_to_ranch_err(e));
    }
    Ok(MaintenanceResult {
        action: op.verb().to_string(),
        affected,
        skipped,
        cancelled,
    })
}
