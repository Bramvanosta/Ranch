//! Pause and resume a queue.

use tokio_util::sync::CancellationToken;

use super::authorize::{authorize_write, write_audit_for_action};
use crate::bullmq_schema::Keys;
use crate::bullmq_schema::QueueRef;
use crate::connection_actor::actor::ActorState;
use crate::connection_actor::link::{
    open_redis_for_actor, redis_to_ranch_err, resolve_keys_for_queue,
};
use ranch_domain::audit::{AuditAction, AuditOutcome, AuditTarget, Caller};
use ranch_domain::errors::RanchError;
use ranch_domain::queues::{QueueActionResult, QueueId};

// Pause / resume queue + scheduler
//
// Pattern (shared by the destructive ops):
//   1. check_lock_or_err(state.lock_state, ...)?     ← safety gate, FIRST line
//   2. Resolve Redis + keys
//   3. Atomic Redis op (MULTI/EXEC where ≥2 keys are touched)
//   4. write_audit_for_action(state, action, target, outcome, reason, caller)  ← LAST step
//   5. Return the resulting state
//
// Idempotency: pause-when-paused / resume-when-running is a
// successful no-op (`already_in_state: true`). The audit row captures
// it via `outcome_reason = "already paused"` / `"already running"`.
//
// BullMQ semantics (researched from BullMQ v5 source):
//   - `Queue.pause()`:  EXISTS :wait + RENAME :wait :paused + HSET :meta paused 1
//   - `Queue.resume()`: EXISTS :paused + RENAME :paused :wait + HDEL :meta paused
//
// BullMQ asks whether the `paused` field exists, not what it holds: a
// queue whose meta still carries `paused`, whatever its value, is paused
// to its workers and to `getTargetQueueList`.
//
// Scheduler pause/resume is Ranch-original (no public BullMQ API for it);
// see the per-handler comments.

/// Pause a queue. Idempotent. See module-level docs for
/// BullMQ semantics.
pub(crate) async fn handle_pause_queue(
    state: &mut ActorState,
    queue_id: &QueueId,
    caller: &Caller,
    token: &CancellationToken,
) -> Result<QueueActionResult, RanchError> {
    let domain_queue = QueueRef::parse(queue_id.as_str())?;
    authorize_write(state, caller, "pause_queue").await?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &domain_queue, token).await?;

    let wait_key = keys.wait(&domain_queue);
    let paused_key = keys.paused(&domain_queue);
    let meta_key = keys.meta(&domain_queue);

    // 1. EXISTS :wait outside the pipeline (MULTI/EXEC doesn't allow
    // conditional logic; we need the value to decide whether to
    // include RENAME).
    let mut exists_cmd = redis::cmd("EXISTS");
    exists_cmd.arg(&wait_key);
    let wait_exists: i64 = tokio::select! {
        r = exists_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };

    // 2. Read the current `paused` flag from :meta; if "1", the
    // call is a no-op (the user/agent re-asked for pause on an
    // already-paused queue). Still HSET the flag below to be defensive.
    let mut hget_cmd = redis::cmd("HGET");
    hget_cmd.arg(&meta_key).arg("paused");
    let current_paused: Option<String> = tokio::select! {
        r = hget_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    let already_in_state = current_paused.as_deref() == Some("1");

    // 3. MULTI/EXEC: conditionally RENAME + always HSET.
    let mut pipe = redis::pipe();
    pipe.atomic();
    if wait_exists == 1 {
        pipe.cmd("RENAME").arg(&wait_key).arg(&paused_key);
    }
    pipe.cmd("HSET").arg(&meta_key).arg("paused").arg("1");
    let result: Result<redis::Value, redis::RedisError> = tokio::select! {
        r = pipe.query_async(&mut conn) => r,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };

    // 4. Audit. Idempotent no-op gets a descriptive outcome_reason.
    let target = AuditTarget::queue(&domain_queue.id(), &domain_queue.name);
    match &result {
        Ok(_) => {
            let reason = if already_in_state {
                Some("already paused".to_string())
            } else {
                None
            };
            write_audit_for_action(
                state,
                AuditAction::PauseQueue,
                target,
                AuditOutcome::Success,
                reason,
                caller,
            )
            .await?;
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::PauseQueue,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                caller,
            )
            .await?;
        }
    };

    result.map_err(redis_to_ranch_err)?;
    Ok(QueueActionResult {
        is_paused: true,
        already_in_state,
    })
}

/// Resume a queue. Idempotent. Symmetric to `handle_pause_queue`.
pub(crate) async fn handle_resume_queue(
    state: &mut ActorState,
    queue_id: &QueueId,
    caller: &Caller,
    token: &CancellationToken,
) -> Result<QueueActionResult, RanchError> {
    let domain_queue = QueueRef::parse(queue_id.as_str())?;
    authorize_write(state, caller, "resume_queue").await?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &domain_queue, token).await?;

    let wait_key = keys.wait(&domain_queue);
    let paused_key = keys.paused(&domain_queue);
    let meta_key = keys.meta(&domain_queue);

    let mut exists_cmd = redis::cmd("EXISTS");
    exists_cmd.arg(&paused_key);
    let paused_exists: i64 = tokio::select! {
        r = exists_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };

    let mut hget_cmd = redis::cmd("HGET");
    hget_cmd.arg(&meta_key).arg("paused");
    let current_paused: Option<String> = tokio::select! {
        r = hget_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    // Running means no `paused` field at all; a leftover `"0"` still
    // pauses the queue for BullMQ, so it is removed like a `"1"`.
    let already_in_state = current_paused.is_none();

    let mut pipe = redis::pipe();
    pipe.atomic();
    if paused_exists == 1 {
        pipe.cmd("RENAME").arg(&paused_key).arg(&wait_key);
    }
    pipe.cmd("HDEL").arg(&meta_key).arg("paused");
    let result: Result<redis::Value, redis::RedisError> = tokio::select! {
        r = pipe.query_async(&mut conn) => r,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };

    let target = AuditTarget::queue(&domain_queue.id(), &domain_queue.name);
    match &result {
        Ok(_) => {
            let reason = if already_in_state {
                Some("already running".to_string())
            } else {
                None
            };
            write_audit_for_action(
                state,
                AuditAction::ResumeQueue,
                target,
                AuditOutcome::Success,
                reason,
                caller,
            )
            .await?;
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::ResumeQueue,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                caller,
            )
            .await?;
        }
    };

    result.map_err(redis_to_ranch_err)?;
    Ok(QueueActionResult {
        is_paused: false,
        already_in_state,
    })
}
