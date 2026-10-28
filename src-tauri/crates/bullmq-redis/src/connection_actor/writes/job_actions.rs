//! Single-job writes: retry a failed job, remove a job, promote a delayed
//! one.

use tokio_util::sync::CancellationToken;

use super::authorize::{authorize_write, write_audit_for_action};
use crate::bullmq_schema::maintenance::{self, MoveCode};
use crate::bullmq_schema::states;
use crate::bullmq_schema::{JobState, KeyVariant, Keys, QueueRef};
use crate::connection_actor::actor::ActorState;
use crate::connection_actor::link::{
    open_redis_for_actor, redis_to_ranch_err, resolve_keys_for_queue, RedisConn,
};
use ranch_domain::audit::{AuditAction, AuditOutcome, AuditTarget, Caller};
use ranch_domain::errors::RanchError;
use ranch_domain::jobs::{JobActionResult, JobTarget};

// Destructive single-job actions (retry / remove / promote)
//
// All three handlers follow the same prescribed shape:
//
//   1. check_lock_or_err(state.lock_state, ...)? ............... safety gate
//   2. open Redis + resolve KeyVariant
//   3. validate state precondition (retry → failed; promote → delayed;
//      remove → any-state, but EXISTS check on the job hash)
//   4. MULTI/EXEC pipeline for the actual mutation
//   5. write_audit(state, action, target, outcome, ...) ........ audit funnel
//   6. return Ok(result)
//
// The lock check is the FIRST line because a locked connection must
// never touch Redis. The audit write is the LAST step before return
// because the audit row must be fsync'd to disk before the
// user sees the action confirmed.
//
// Locked-connection rejections return BEFORE the audit step: for
// single-action paths we deliberately skip auditing the
// rejection (signal-to-noise on single attempts is low; the bulk
// paths write a consolidated "attempted-on-locked" audit row).

/// Aliased so clippy's `type_complexity` lint stays quiet: the 9
/// pipelined membership probes (3 LPOS + 6 ZSCORE) return a wide
/// tuple by design.
type StateProbe = (
    Option<i64>, // wait (LPOS)
    Option<i64>, // active (LPOS)
    Option<i64>, // paused (LPOS)
    Option<f64>, // completed (ZSCORE)
    Option<f64>, // failed
    Option<f64>, // delayed
    Option<f64>, // prioritized
    Option<f64>, // waiting-children
    Option<f64>, // stalled
);

/// Detect which BullMQ state the job currently sits in, returning a
/// lowercase camelCase string ready for the `InvalidJobState` error
/// payload. Used by the precondition-violation path so the error
/// message reads naturally ("current state is waiting, required failed").
///
/// Implementation: one pipelined RTT covering the 3 list states
/// (LPOS) + 6 sorted-set states (ZSCORE). The first hit wins. Not
/// finding the job in any container returns `"unknown"`; the
/// caller decides whether that's a `JobNotFound` (no hash) or just
/// a mid-state race.
pub(super) async fn detect_job_state(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    job_id: &str,
    token: &CancellationToken,
) -> Result<JobState, RanchError> {
    let wait_key = keys.wait(qref);
    let active_key = keys.active(qref);
    let paused_key = keys.paused(qref);
    let completed_key = keys.completed(qref);
    let failed_key = keys.failed(qref);
    let delayed_key = keys.delayed(qref);
    let prioritized_key = keys.prioritized(qref);
    let waiting_children_key = keys.waiting_children(qref);
    let stalled_key = keys.stalled(qref);

    let mut pipe = redis::pipe();
    pipe.cmd("LPOS")
        .arg(&wait_key)
        .arg(job_id)
        .cmd("LPOS")
        .arg(&active_key)
        .arg(job_id)
        .cmd("LPOS")
        .arg(&paused_key)
        .arg(job_id)
        .cmd("ZSCORE")
        .arg(&completed_key)
        .arg(job_id)
        .cmd("ZSCORE")
        .arg(&failed_key)
        .arg(job_id)
        .cmd("ZSCORE")
        .arg(&delayed_key)
        .arg(job_id)
        .cmd("ZSCORE")
        .arg(&prioritized_key)
        .arg(job_id)
        .cmd("ZSCORE")
        .arg(&waiting_children_key)
        .arg(job_id)
        .cmd("ZSCORE")
        .arg(&stalled_key)
        .arg(job_id);
    let (
        lpos_wait,
        lpos_active,
        lpos_paused,
        z_completed,
        z_failed,
        z_delayed,
        z_prioritized,
        z_wc,
        z_stalled,
    ): StateProbe = tokio::select! {
        r = pipe.query_async(conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };

    let state = if lpos_wait.is_some() {
        JobState::Waiting
    } else if lpos_active.is_some() {
        JobState::Active
    } else if lpos_paused.is_some() {
        JobState::Paused
    } else if z_failed.is_some() {
        JobState::Failed
    } else if z_delayed.is_some() {
        JobState::Delayed
    } else if z_completed.is_some() {
        JobState::Completed
    } else if z_prioritized.is_some() {
        states::PRIORITIZED
    } else if z_wc.is_some() {
        states::WAITING_CHILDREN
    } else if z_stalled.is_some() {
        states::STALLED
    } else {
        states::UNKNOWN
    };
    Ok(state)
}

/// Run `retry_jobs.lua` for `ids`: one [`MoveCode`] per id.
///
/// Replaces a plain `MULTI/EXEC`, which pushed straight into `wait`:
/// on a **paused** queue BullMQ's resume `RENAME`s `paused` over `wait`, so
/// the retried job vanished from every list; idle workers were not woken
/// (no marker); no `waiting` event; and it wrote `failedReason ""` /
/// `attemptsMade 0`, fields BullMQ does not read that way. The script does
/// what BullMQ's `reprocessJob` does.
pub(super) async fn run_retry_script(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    ids: &[String],
) -> Result<Vec<MoveCode>, redis::RedisError> {
    let script = redis::Script::new(&maintenance::retry_jobs_lua());
    let mut invocation = script.prepare_invoke();
    invocation
        .key(keys.failed(qref))
        .key(keys.wait(qref))
        .key(keys.paused(qref))
        .key(keys.meta(qref))
        .key(keys.active(qref))
        .key(keys.events(qref))
        .key(keys.marker(qref))
        .arg(format!("{}:", keys.queue_base(qref)));
    for id in ids {
        invocation.arg(id);
    }
    let codes: Vec<i64> = invocation.invoke_async(conn).await?;
    Ok(codes.into_iter().map(MoveCode::from_script).collect())
}

/// Run `promote_jobs.lua` for `ids`. Same reason as
/// [`run_retry_script`]: the old `MULTI/EXEC` pushed into `wait` whatever the
/// queue's state, ignored the job's priority and woke nobody.
pub(super) async fn run_promote_script(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    ids: &[String],
) -> Result<Vec<MoveCode>, redis::RedisError> {
    let script = redis::Script::new(&maintenance::promote_jobs_lua());
    let mut invocation = script.prepare_invoke();
    invocation
        .key(keys.delayed(qref))
        .key(keys.wait(qref))
        .key(keys.paused(qref))
        .key(keys.meta(qref))
        .key(keys.active(qref))
        .key(keys.events(qref))
        .key(keys.marker(qref))
        .key(keys.prioritized(qref))
        .key(keys.priority_counter(qref))
        .arg(format!("{}:", keys.queue_base(qref)));
    for id in ids {
        invocation.arg(id);
    }
    let codes: Vec<i64> = invocation.invoke_async(conn).await?;
    Ok(codes.into_iter().map(MoveCode::from_script).collect())
}

/// Retry a failed job. The write is
/// `retry_jobs.lua` (see [`run_retry_script`]): back in `wait` (or `paused`)
/// with its failure erased, workers woken, a `waiting` event emitted.
///
/// State precondition: job MUST be in `failed` state. Other states
/// surface `RanchError::InvalidJobState`. The frontend's button
/// disable is cosmetic; this check is the actual guard.
pub(crate) async fn handle_retry_job(
    state: &mut ActorState,
    args: &JobTarget,
    caller: &Caller,
    token: &CancellationToken,
) -> Result<JobActionResult, RanchError> {
    let queue = QueueRef::parse(args.queue_id.as_str())?;
    // 1. SAFETY GATE, first line. A locked connection never touches Redis.
    authorize_write(state, caller, "retry_job").await?;

    // 2. Open Redis + resolve keys.
    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &queue, token).await?;

    // 3. Validate state precondition: job IS in `failed`.
    let failed_key = keys.failed(&queue);
    let mut zscore_cmd = redis::cmd("ZSCORE");
    zscore_cmd.arg(&failed_key).arg(&args.job_id);
    let in_failed: Option<f64> = tokio::select! {
        r = zscore_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    if in_failed.is_none() {
        let current = detect_job_state(&mut conn, keys, &queue, &args.job_id, token).await?;
        // Precondition failure is NOT audited (the lock sentinel covers
        // the lock-rejection case; state-precondition rejections are
        // just a UI sync issue and would noise up the audit log).
        return Err(RanchError::InvalidJobState {
            action: "retry".into(),
            connection_id: state.cfg.connection_id.clone(),
            queue_name: queue.name.clone(),
            job_id: args.job_id.clone(),
            current_state: current,
            required_state: JobState::Failed,
        });
    }

    // 4. Atomic retry: one script (see `run_retry_script`).
    let ids = [args.job_id.clone()];
    let result: Result<Vec<MoveCode>, redis::RedisError> = tokio::select! {
        r = run_retry_script(&mut conn, keys, &queue, &ids) => r,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    // Lost a race between the check above and the script (a worker or
    // another client moved the job): same answer as the precondition.
    if let Ok(codes) = &result {
        if codes.first() != Some(&MoveCode::Moved) {
            let current = detect_job_state(&mut conn, keys, &queue, &args.job_id, token).await?;
            return Err(RanchError::InvalidJobState {
                action: "retry".into(),
                connection_id: state.cfg.connection_id.clone(),
                queue_name: queue.name.clone(),
                job_id: args.job_id.clone(),
                current_state: current,
                required_state: JobState::Failed,
            });
        }
    }

    // 5. Audit (success OR failure), last step before return.
    let target = AuditTarget::job(&queue.id(), &queue.name, &args.job_id);
    match &result {
        Ok(_) => {
            write_audit_for_action(
                state,
                AuditAction::RetryJob,
                target,
                AuditOutcome::Success,
                None,
                caller,
            )
            .await?;
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::RetryJob,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                caller,
            )
            .await?;
        }
    };

    result.map_err(redis_to_ranch_err)?;
    Ok(JobActionResult {
        job_id: args.job_id.clone(),
        new_state: Some(JobState::Waiting),
        action: "retry".into(),
    })
}

/// Remove a job from every state container + the job hash + logs.
///
/// Algorithm:
///   1. EXISTS bull:{q}:jobId → JobNotFound if 0.
///   2. detect_job_state → which container holds the job (if any).
///   3. MULTI/EXEC: remove from that container (LREM or ZREM) + DEL
///      job_hash + DEL logs + DEL lock.
///   4. Audit.
pub(crate) async fn handle_remove_job(
    state: &mut ActorState,
    args: &JobTarget,
    caller: &Caller,
    token: &CancellationToken,
) -> Result<JobActionResult, RanchError> {
    let domain_queue = QueueRef::parse(args.queue_id.as_str())?;
    authorize_write(state, caller, "remove_job").await?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &domain_queue, token).await?;

    let jref = domain_queue.job(args.job_id.clone());
    let job_hash_key = keys.job_hash(&jref);
    let logs_key = keys.logs(&jref);
    let lock_key = format!("{}:lock", job_hash_key);

    // 1. EXISTS guard.
    let mut exists_cmd = redis::cmd("EXISTS");
    exists_cmd.arg(&job_hash_key);
    let exists: i64 = tokio::select! {
        r = exists_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    if exists == 0 {
        return Err(RanchError::JobNotFound {
            queue: domain_queue.name.clone(),
            id: args.job_id.clone(),
        });
    }

    // 2. Detect the state container so we know which key to remove from.
    let detected = detect_job_state(&mut conn, keys, &domain_queue, &args.job_id, token).await?;

    // 3. MULTI/EXEC: container-removal + hash + logs + lock.
    let mut pipe = redis::pipe();
    pipe.atomic();
    match detected.as_str() {
        "waiting" => {
            pipe.cmd("LREM")
                .arg(keys.wait(&domain_queue))
                .arg(0)
                .arg(&args.job_id);
        }
        "active" => {
            pipe.cmd("LREM")
                .arg(keys.active(&domain_queue))
                .arg(0)
                .arg(&args.job_id);
        }
        "paused" => {
            pipe.cmd("LREM")
                .arg(keys.paused(&domain_queue))
                .arg(0)
                .arg(&args.job_id);
        }
        "completed" => {
            pipe.cmd("ZREM")
                .arg(keys.completed(&domain_queue))
                .arg(&args.job_id);
        }
        "failed" => {
            pipe.cmd("ZREM")
                .arg(keys.failed(&domain_queue))
                .arg(&args.job_id);
        }
        "delayed" => {
            pipe.cmd("ZREM")
                .arg(keys.delayed(&domain_queue))
                .arg(&args.job_id);
        }
        "prioritized" => {
            pipe.cmd("ZREM")
                .arg(keys.prioritized(&domain_queue))
                .arg(&args.job_id);
        }
        "waitingChildren" => {
            pipe.cmd("ZREM")
                .arg(keys.waiting_children(&domain_queue))
                .arg(&args.job_id);
        }
        "stalled" => {
            pipe.cmd("ZREM")
                .arg(keys.stalled(&domain_queue))
                .arg(&args.job_id);
        }
        _ => {
            // Hash exists but no container holds the id: a very rare
            // mid-state race. Still delete the hash + logs.
            // (Add a no-op so the pipeline is non-empty.)
            pipe.cmd("PING");
        }
    }
    pipe.cmd("DEL").arg(&job_hash_key);
    pipe.cmd("DEL").arg(&logs_key);
    pipe.cmd("DEL").arg(&lock_key);

    let result: Result<redis::Value, redis::RedisError> = tokio::select! {
        r = pipe.query_async(&mut conn) => r,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };

    let target = AuditTarget::job(&domain_queue.id(), &domain_queue.name, &args.job_id);
    match &result {
        Ok(_) => {
            write_audit_for_action(
                state,
                AuditAction::RemoveJob,
                target,
                AuditOutcome::Success,
                None,
                caller,
            )
            .await?;
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::RemoveJob,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                caller,
            )
            .await?;
        }
    };

    result.map_err(redis_to_ranch_err)?;
    Ok(JobActionResult {
        job_id: args.job_id.clone(),
        new_state: None,
        action: "remove".into(),
    })
}

/// Promote a delayed job into the waiting list.
///
/// Atomic semantics (single MULTI/EXEC):
///   - `ZREM bull:{q}:delayed jobId`
///   - `HSET bull:{q}:jobId delay 0`
///   - `LPUSH bull:{q}:wait jobId`
pub(crate) async fn handle_promote_job(
    state: &mut ActorState,
    args: &JobTarget,
    caller: &Caller,
    token: &CancellationToken,
) -> Result<JobActionResult, RanchError> {
    let queue = QueueRef::parse(args.queue_id.as_str())?;
    authorize_write(state, caller, "promote_job").await?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &queue, token).await?;

    // Validate state precondition: job IS in `delayed`.
    let delayed_key = keys.delayed(&queue);
    let mut zscore_cmd = redis::cmd("ZSCORE");
    zscore_cmd.arg(&delayed_key).arg(&args.job_id);
    let in_delayed: Option<f64> = tokio::select! {
        r = zscore_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    if in_delayed.is_none() {
        let current = detect_job_state(&mut conn, keys, &queue, &args.job_id, token).await?;
        return Err(RanchError::InvalidJobState {
            action: "promote".into(),
            connection_id: state.cfg.connection_id.clone(),
            queue_name: queue.name.clone(),
            job_id: args.job_id.clone(),
            current_state: current,
            required_state: JobState::Delayed,
        });
    }

    // One script (see `run_promote_script`).
    let ids = [args.job_id.clone()];
    let result: Result<Vec<MoveCode>, redis::RedisError> = tokio::select! {
        r = run_promote_script(&mut conn, keys, &queue, &ids) => r,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    if let Ok(codes) = &result {
        if codes.first() != Some(&MoveCode::Moved) {
            let current = detect_job_state(&mut conn, keys, &queue, &args.job_id, token).await?;
            return Err(RanchError::InvalidJobState {
                action: "promote".into(),
                connection_id: state.cfg.connection_id.clone(),
                queue_name: queue.name.clone(),
                job_id: args.job_id.clone(),
                current_state: current,
                required_state: JobState::Delayed,
            });
        }
    }

    let target = AuditTarget::job(&queue.id(), &queue.name, &args.job_id);
    match &result {
        Ok(_) => {
            write_audit_for_action(
                state,
                AuditAction::PromoteJob,
                target,
                AuditOutcome::Success,
                None,
                caller,
            )
            .await?;
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::PromoteJob,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                caller,
            )
            .await?;
        }
    };

    result.map_err(redis_to_ranch_err)?;
    Ok(JobActionResult {
        job_id: args.job_id.clone(),
        new_state: Some(JobState::Waiting),
        action: "promote".into(),
    })
}
