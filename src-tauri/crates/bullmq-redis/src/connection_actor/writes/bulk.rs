//! Bulk single-queue writes (retry / remove / promote many jobs): one
//! gate, per-job steps, progress events, one consolidated audit row.

use tokio_util::sync::CancellationToken;

use super::authorize::{authorize_write, write_audit_for_action};
use super::job_actions::{detect_job_state, run_promote_script, run_retry_script};
use crate::bullmq_schema::maintenance::MoveCode;
use crate::bullmq_schema::{KeyVariant, Keys, QueueRef};
use crate::connection_actor::actor::ActorState;
use crate::connection_actor::link::{open_redis_for_actor, resolve_keys_for_queue, RedisConn};
use ranch_application::ports::ProgressSink;
use ranch_domain::audit::{AuditAction, AuditOutcome, AuditTarget, Caller};
use ranch_domain::errors::RanchError;
use ranch_domain::jobs::{BulkActionResult, BulkFailure, BulkProgress, JobBatch};

// Bulk single-queue destructive actions
//
// Each bulk handler follows the prescribed shape:
//
//   1. check_lock_or_err(state.lock_state, ...)? ............... ONE safety gate
//   2. open Redis once + resolve KeyVariant once
//   3. for each job_id in args.job_ids:
//        a. check token.is_cancelled(); break if so
//        b. call the matching `*_one_for_bulk` helper (per-job MULTI/EXEC)
//        c. accumulate Ok → succeeded; Err(reason) → failed[]
//        d. every BULK_PROGRESS_CHUNK jobs OR on the last job, emit
//           progress to the sink (`bulk:progress` on the desktop)
//   4. write_audit_for_action(state, action, BulkJobs{...}, outcome, reason, caller) ... ONE consolidated audit
//   5. return BulkActionResult
//
// The lock check is ONCE at the top: the bulk run captures the lock
// state at start. By design, re-locking mid-bulk doesn't abort
// in-flight ops. Per-job errors don't abort the loop; only mid-bulk
// cancellation (token.is_cancelled() = true) or pre-loop failures
// (open_redis, resolve_keys) break out.

/// Emit a `bulk:progress` event every N completed jobs
/// AND on the last job. Chunk size of 10 keeps the progress bar
/// smooth without flooding the event bus (247 events → 25 events).
const BULK_PROGRESS_CHUNK: usize = 10;

/// per-job retry helper used by `handle_bulk_retry_jobs`.
/// Returns `Ok(())` on success, `Err(reason)` on any failure
/// (precondition or Redis). The bulk handler collects the reason
/// into a `BulkFailure` entry.
///
/// State precondition: job MUST be in `failed`. The check is per-job
/// (not pre-flight) so the bulk can run on a mixed selection and
/// surface state-mismatch jobs as failures rather than rejecting
/// the whole call.
async fn retry_one_for_bulk(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    job_id: &str,
    token: &CancellationToken,
) -> Result<(), String> {
    let ids = [job_id.to_string()];
    let codes = tokio::select! {
        r = run_retry_script(conn, keys, qref, &ids) => r.map_err(|e| e.to_string())?,
        _ = token.cancelled() => return Err("cancelled".to_string()),
    };
    match codes.first() {
        Some(MoveCode::Moved) => Ok(()),
        Some(MoveCode::WrongState) => Err("not in failed state".to_string()),
        _ => Err("job not found".to_string()),
    }
}

/// per-job remove helper used by `handle_bulk_remove_jobs`.
/// Mirrors the single-job `handle_remove_job` Redis op sequence
/// (EXISTS check + detect-state + MULTI/EXEC).
async fn remove_one_for_bulk(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    job_id: &str,
    token: &CancellationToken,
) -> Result<(), String> {
    let jref = qref.job(job_id.to_string());
    let job_hash_key = keys.job_hash(&jref);
    let logs_key = keys.logs(&jref);
    let lock_key = format!("{}:lock", job_hash_key);

    let mut exists_cmd = redis::cmd("EXISTS");
    exists_cmd.arg(&job_hash_key);
    let exists: i64 = tokio::select! {
        r = exists_cmd.query_async(conn) => r.map_err(|e| e.to_string())?,
        _ = token.cancelled() => return Err("cancelled".to_string()),
    };
    if exists == 0 {
        return Err("job not found".to_string());
    }

    let detected = detect_job_state(conn, keys, qref, job_id, token)
        .await
        .map_err(|e| e.to_string())?;

    let mut pipe = redis::pipe();
    pipe.atomic();
    match detected.as_str() {
        "waiting" => {
            pipe.cmd("LREM").arg(keys.wait(qref)).arg(0).arg(job_id);
        }
        "active" => {
            pipe.cmd("LREM").arg(keys.active(qref)).arg(0).arg(job_id);
        }
        "paused" => {
            pipe.cmd("LREM").arg(keys.paused(qref)).arg(0).arg(job_id);
        }
        "completed" => {
            pipe.cmd("ZREM").arg(keys.completed(qref)).arg(job_id);
        }
        "failed" => {
            pipe.cmd("ZREM").arg(keys.failed(qref)).arg(job_id);
        }
        "delayed" => {
            pipe.cmd("ZREM").arg(keys.delayed(qref)).arg(job_id);
        }
        "prioritized" => {
            pipe.cmd("ZREM").arg(keys.prioritized(qref)).arg(job_id);
        }
        "waitingChildren" => {
            pipe.cmd("ZREM")
                .arg(keys.waiting_children(qref))
                .arg(job_id);
        }
        "stalled" => {
            pipe.cmd("ZREM").arg(keys.stalled(qref)).arg(job_id);
        }
        _ => {
            pipe.cmd("PING");
        }
    }
    pipe.cmd("DEL").arg(&job_hash_key);
    pipe.cmd("DEL").arg(&logs_key);
    pipe.cmd("DEL").arg(&lock_key);

    let _: redis::Value = tokio::select! {
        r = pipe.query_async(conn) => r.map_err(|e| e.to_string())?,
        _ = token.cancelled() => return Err("cancelled".to_string()),
    };
    Ok(())
}

/// per-job promote helper used by `handle_bulk_promote_jobs`.
async fn promote_one_for_bulk(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    job_id: &str,
    token: &CancellationToken,
) -> Result<(), String> {
    let ids = [job_id.to_string()];
    let codes = tokio::select! {
        r = run_promote_script(conn, keys, qref, &ids) => r.map_err(|e| e.to_string())?,
        _ = token.cancelled() => return Err("cancelled".to_string()),
    };
    match codes.first() {
        Some(MoveCode::Moved) => Ok(()),
        Some(MoveCode::WrongState) => Err("not in delayed state".to_string()),
        _ => Err("job not found".to_string()),
    }
}

/// Format the `outcome_reason` for a bulk audit row. Summarizes
/// success / fail / cancel counts and lists up to the first 5 job
/// failures by id + reason.
fn format_bulk_reason(
    succeeded: u32,
    total: u32,
    failed: &[BulkFailure],
    cancelled: bool,
) -> String {
    let mut s = if cancelled {
        format!(
            "Cancelled: {succeeded} of {total} succeeded before stop, {} failed",
            failed.len()
        )
    } else {
        format!("{succeeded} of {total} succeeded, {} failed", failed.len())
    };
    if !failed.is_empty() {
        s.push_str(": ");
        let preview: Vec<String> = failed
            .iter()
            .take(5)
            .map(|f| format!("{} ({})", f.job_id, f.reason))
            .collect();
        s.push_str(&preview.join(", "));
        if failed.len() > 5 {
            s.push_str(&format!(", … and {} more", failed.len() - 5));
        }
    }
    s
}

/// Inline the bulk-loop body. Each of the 3 bulk handlers calls this
/// macro with the matching per-job helper. We use a macro rather than
/// a generic `Fn` parameter because async-fn-pointers under HRTB
/// (`for<'a> Fn(...)`) don't elaborate lifetimes correctly in
/// rustc 1.95; see the rustc note about "must implement Fn<(&'a)>
/// but actually implements Fn<(&'0)> for some specific lifetime".
/// Macro avoids the type-system limitation; the duplication is
/// confined to two lines per handler.
macro_rules! bulk_handler_body {
    ($state:ident, $args:ident, $caller:ident, $progress:ident, $token:ident, $tool:expr, $verb:expr, $audit_action:expr, $op_fn:ident) => {{
        let queue = QueueRef::parse($args.queue_id.as_str())?;
        // 1. ONE authorization at the top: the lock decision, or the
        // human approval, applies to the whole bulk.
        authorize_write(
            $state,
            $caller,
            $tool,
            $audit_action,
            AuditTarget::bulk_jobs(&queue.id(), &queue.name, $args.job_ids.len() as u32),
        )
        .await?;

        // 2. Open Redis once + resolve keys once.
        let mut conn = open_redis_for_actor($state, $token).await?;
        let keys = resolve_keys_for_queue($state, &mut conn, &queue, $token).await?;

        let total = $args.job_ids.len() as u32;
        let mut succeeded: u32 = 0;
        let mut failed: Vec<BulkFailure> = Vec::new();
        let mut cancelled = false;

        for (i, job_id) in $args.job_ids.iter().enumerate() {
            if $token.is_cancelled() {
                cancelled = true;
                break;
            }
            match $op_fn(&mut conn, keys, &queue, job_id, $token).await {
                Ok(()) => succeeded += 1,
                Err(reason) => failed.push(BulkFailure {
                    job_id: job_id.clone(),
                    reason,
                }),
            }
            let done_count = i + 1;
            if done_count % BULK_PROGRESS_CHUNK == 0 || done_count == $args.job_ids.len() {
                $progress.bulk(BulkProgress {
                    action: $verb.to_string(),
                    queue_id: queue.id(),
                    total,
                    completed: succeeded + failed.len() as u32,
                    failed_so_far: failed.len() as u32,
                });
            }
        }

        // 3. ONE consolidated audit row.
        let outcome = if cancelled {
            AuditOutcome::Cancelled
        } else if failed.is_empty() {
            AuditOutcome::Success
        } else {
            AuditOutcome::Failure
        };
        let reason = if cancelled || !failed.is_empty() {
            Some(format_bulk_reason(succeeded, total, &failed, cancelled))
        } else {
            None
        };
        write_audit_for_action(
            $state,
            $audit_action,
            AuditTarget::bulk_jobs(&queue.id(), &queue.name, total),
            outcome,
            reason,
            $caller,
        )
        .await?;

        Ok(BulkActionResult {
            action: $verb.to_string(),
            total,
            succeeded,
            failed,
            cancelled,
        })
    }};
}

pub(crate) async fn handle_bulk_retry_jobs(
    state: &mut ActorState,
    args: &JobBatch,
    caller: &Caller,
    progress: &dyn ProgressSink,
    token: &CancellationToken,
) -> Result<BulkActionResult, RanchError> {
    bulk_handler_body!(
        state,
        args,
        caller,
        progress,
        token,
        "bulk_retry_jobs",
        "retry",
        AuditAction::RetryJob,
        retry_one_for_bulk
    )
}

pub(crate) async fn handle_bulk_remove_jobs(
    state: &mut ActorState,
    args: &JobBatch,
    caller: &Caller,
    progress: &dyn ProgressSink,
    token: &CancellationToken,
) -> Result<BulkActionResult, RanchError> {
    bulk_handler_body!(
        state,
        args,
        caller,
        progress,
        token,
        "bulk_remove_jobs",
        "remove",
        AuditAction::RemoveJob,
        remove_one_for_bulk
    )
}

pub(crate) async fn handle_bulk_promote_jobs(
    state: &mut ActorState,
    args: &JobBatch,
    caller: &Caller,
    progress: &dyn ProgressSink,
    token: &CancellationToken,
) -> Result<BulkActionResult, RanchError> {
    bulk_handler_body!(
        state,
        args,
        caller,
        progress,
        token,
        "bulk_promote_jobs",
        "promote",
        AuditAction::PromoteJob,
        promote_one_for_bulk
    )
}
