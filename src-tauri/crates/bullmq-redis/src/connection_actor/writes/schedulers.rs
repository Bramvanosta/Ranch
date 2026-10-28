//! Job scheduler writes: create or replace, remove, pause and resume.

use std::collections::HashMap;

use tokio_util::sync::CancellationToken;

use super::authorize::{authorize_write, write_audit_for_action};
use crate::bullmq_schema::scheduler;
use crate::bullmq_schema::{KeyVariant, Keys, QueueRef, SchedulerSaved, V5Keys};
use crate::connection_actor::actor::ActorState;
use crate::connection_actor::link::{
    open_redis_for_actor, redis_to_ranch_err, resolve_keys_for_queue, RedisConn,
};
use ranch_domain::audit::{AuditAction, AuditOutcome, AuditTarget, Caller};
use ranch_domain::errors::RanchError;
use ranch_domain::queues::QueueId;
use ranch_domain::schedule::{
    PreparedScheduler, Schedule, SchedulerActionResult, SchedulerSpec, SchedulerTarget,
};

/// The scheduler's settings, read back from its hash.
async fn read_scheduler_spec(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    scheduler_key: &str,
) -> Result<Option<(SchedulerSpec, bool)>, redis::RedisError> {
    let hash_key = format!("{}:{}", keys.repeat(qref), scheduler_key);
    let fields: HashMap<String, String> = redis::cmd("HGETALL")
        .arg(&hash_key)
        .query_async(conn)
        .await?;
    let Some(name) = fields.get("name") else {
        return Ok(None);
    };
    let schedule = match (fields.get("pattern"), fields.get("every")) {
        (Some(pattern), _) => Schedule::Pattern {
            pattern: pattern.clone(),
            tz: fields.get("tz").cloned(),
        },
        (None, Some(every)) => Schedule::Every {
            every_ms: every.parse().unwrap_or(0),
        },
        (None, None) => return Ok(None),
    };
    let paused = fields.get("paused").map(String::as_str) == Some("1");
    Ok(Some((
        SchedulerSpec {
            id: scheduler_key.to_string(),
            job_name: name.clone(),
            schedule,
            data: fields.get("data").cloned().unwrap_or_else(|| "{}".into()),
        },
        paused,
    )))
}

/// Run `upsert_scheduler.lua`.
async fn run_upsert_scheduler_script(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    prepared: &PreparedScheduler,
    now_ms: i64,
) -> Result<Result<SchedulerSaved, String>, redis::RedisError> {
    let opt = |v: Option<String>| v.unwrap_or_default();
    let (job_id, second): (String, i64) = redis::Script::new(&scheduler::upsert_scheduler_lua())
        .key(keys.repeat(qref))
        .key(keys.delayed(qref))
        .key(keys.wait(qref))
        .key(keys.paused(qref))
        .key(keys.meta(qref))
        .key(keys.prioritized(qref))
        .key(keys.marker(qref))
        .key(keys.id_counter(qref))
        .key(keys.events(qref))
        .key(keys.priority_counter(qref))
        .key(keys.active(qref))
        .arg(format!("{}:", keys.queue_base(qref)))
        .arg(&prepared.id)
        .arg(&prepared.job_name)
        .arg(opt(prepared.pattern.clone()))
        .arg(opt(prepared.tz.clone()))
        .arg(opt(prepared.every_ms.map(|e| e.to_string())))
        .arg(opt(prepared.first_run_ms.map(|m| m.to_string())))
        .arg(now_ms)
        .arg(&prepared.data)
        .invoke_async(conn)
        .await?;
    if job_id.is_empty() {
        return Ok(Err(match second {
            -10 => "A job already exists for the next run of this pattern (it is running or has just finished). Try again in a moment.".to_string(),
            -11 => "Every upcoming slot of this interval already has a job. Remove the stale `repeat:` jobs of this scheduler first.".to_string(),
            code => format!("The scheduler script refused the change (code {code})"),
        }));
    }
    Ok(Ok(SchedulerSaved {
        scheduler_id: prepared.id.clone(),
        next_run_ms: now_ms + second,
        job_id,
    }))
}

/// Create or replace a job scheduler: gate → validate
/// (pure; the first run of a pattern is computed here, in the scheduler's
/// timezone) → queue must exist → `upsert_scheduler.lua` → audit.
pub(crate) async fn handle_upsert_scheduler(
    state: &mut ActorState,
    queue_id: &QueueId,
    spec: &SchedulerSpec,
    caller: &Caller,
    token: &CancellationToken,
) -> Result<SchedulerSaved, RanchError> {
    authorize_write(state, caller, "upsert_scheduler").await?;

    let now = chrono::Utc::now();
    let prepared = spec
        .prepare(now)
        .map_err(|reason| RanchError::InvalidInput { reason })?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let q = &QueueRef::parse(queue_id.as_str())?;
    let mut exists_cmd = redis::cmd("EXISTS");
    exists_cmd.arg(V5Keys.meta(q));
    let queue_exists: bool = tokio::select! {
        r = exists_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    if !queue_exists {
        return Err(RanchError::InvalidInput {
            reason: format!(
                "Queue \"{}\" does not exist on this connection (prefix \"{}\")",
                q.name, q.prefix
            ),
        });
    }
    let keys = resolve_keys_for_queue(state, &mut conn, q, token).await?;

    let result = tokio::select! {
        r = run_upsert_scheduler_script(&mut conn, keys, q, &prepared, now.timestamp_millis()) => r,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    let target = AuditTarget::scheduler(&q.id(), &q.name, &prepared.id);
    let described = match (&prepared.pattern, prepared.every_ms) {
        (Some(p), _) => format!(
            "{} · {p}{}",
            prepared.job_name,
            prepared
                .tz
                .as_deref()
                .map(|tz| format!(" ({tz})"))
                .unwrap_or_default()
        ),
        (None, Some(every)) => format!("{} · every {every} ms", prepared.job_name),
        (None, None) => prepared.job_name.clone(),
    };
    match result {
        Ok(Ok(saved)) => {
            write_audit_for_action(
                state,
                AuditAction::UpsertScheduler,
                target,
                AuditOutcome::Success,
                Some(described),
                caller,
            )
            .await?;
            Ok(saved)
        }
        Ok(Err(refusal)) => {
            write_audit_for_action(
                state,
                AuditAction::UpsertScheduler,
                target,
                AuditOutcome::Failure,
                Some(refusal.clone()),
                caller,
            )
            .await?;
            Err(RanchError::InvalidInput { reason: refusal })
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::UpsertScheduler,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                caller,
            )
            .await?;
            Err(redis_to_ranch_err(e))
        }
    }
}

/// `remove_scheduler.lua`: `mode` = "remove" | "pause".
/// Returns `None` when the scheduler does not exist.
async fn run_remove_scheduler_script(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    scheduler_key: &str,
    mode: &str,
) -> Result<Option<bool>, redis::RedisError> {
    let (code, removed_pending): (i64, i64) =
        redis::Script::new(&scheduler::remove_scheduler_lua())
            .key(keys.repeat(qref))
            .key(keys.delayed(qref))
            .key(keys.events(qref))
            .key(keys.wait(qref))
            .key(keys.paused(qref))
            .key(keys.prioritized(qref))
            .arg(scheduler_key)
            .arg(format!("{}:", keys.queue_base(qref)))
            .arg(mode)
            .invoke_async(conn)
            .await?;
    Ok((code == 0).then_some(removed_pending == 1))
}

/// Delete a job scheduler and its pending job. A job of
/// it that is running finishes; it has no successor.
pub(crate) async fn handle_remove_scheduler(
    state: &mut ActorState,
    args: &SchedulerTarget,
    caller: &Caller,
    token: &CancellationToken,
) -> Result<SchedulerActionResult, RanchError> {
    let queue = QueueRef::parse(args.queue_id.as_str())?;
    authorize_write(state, caller, "remove_scheduler").await?;
    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &queue, token).await?;

    let result = tokio::select! {
        r = run_remove_scheduler_script(&mut conn, keys, &queue, &args.scheduler_key, "remove") => r,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    let target = AuditTarget::scheduler(&queue.id(), &queue.name, &args.scheduler_key);
    match result {
        // Not audited: nothing was attempted (same convention as the other destructive ops).
        Ok(None) => Err(RanchError::SchedulerNotFound {
            connection_id: state.cfg.connection_id.clone(),
            queue_name: queue.name.clone(),
            scheduler_key: args.scheduler_key.clone(),
        }),
        Ok(Some(removed_pending)) => {
            write_audit_for_action(
                state,
                AuditAction::RemoveScheduler,
                target,
                AuditOutcome::Success,
                Some(if removed_pending {
                    "pending job removed".to_string()
                } else {
                    "no pending job (running or already gone)".to_string()
                }),
                caller,
            )
            .await?;
            Ok(SchedulerActionResult {
                paused: false,
                already_in_state: false,
            })
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::RemoveScheduler,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                caller,
            )
            .await?;
            Err(redis_to_ranch_err(e))
        }
    }
}

/// Pause a job scheduler.
///
/// BullMQ has no such notion, and no spawner either: the worker that
/// finishes a scheduler's pending job creates the next one. So pausing =
/// **removing the pending job** (`remove_scheduler.lua`, mode `pause`) and
/// flagging the hash `paused = 1`. The scheduler stays in `repeat`, hence
/// stays listed, by Ranch and by BullMQ.
///
/// (Removing the scheduler from `repeat` instead would make it vanish from
/// Ranch's own list (which reads `repeat`), so it could not be resumed from
/// the UI, and its pending job would still run once.)
pub(crate) async fn handle_pause_scheduler(
    state: &mut ActorState,
    args: &SchedulerTarget,
    caller: &Caller,
    token: &CancellationToken,
) -> Result<SchedulerActionResult, RanchError> {
    let queue = QueueRef::parse(args.queue_id.as_str())?;
    authorize_write(state, caller, "pause_scheduler").await?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &queue, token).await?;
    let not_found = |state: &ActorState| RanchError::SchedulerNotFound {
        connection_id: state.cfg.connection_id.clone(),
        queue_name: queue.name.clone(),
        scheduler_key: args.scheduler_key.clone(),
    };

    let spec = tokio::select! {
        r = read_scheduler_spec(&mut conn, keys, &queue, &args.scheduler_key) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    let Some((_, already_paused)) = spec else {
        return Err(not_found(state));
    };

    let target = AuditTarget::scheduler(&queue.id(), &queue.name, &args.scheduler_key);
    if already_paused {
        write_audit_for_action(
            state,
            AuditAction::PauseScheduler,
            target,
            AuditOutcome::Success,
            Some("already paused".to_string()),
            caller,
        )
        .await?;
        return Ok(SchedulerActionResult {
            paused: true,
            already_in_state: true,
        });
    }

    let result = tokio::select! {
        r = run_remove_scheduler_script(&mut conn, keys, &queue, &args.scheduler_key, "pause") => r,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    match result {
        Ok(None) => Err(not_found(state)),
        Ok(Some(_)) => {
            write_audit_for_action(
                state,
                AuditAction::PauseScheduler,
                target,
                AuditOutcome::Success,
                None,
                caller,
            )
            .await?;
            Ok(SchedulerActionResult {
                paused: true,
                already_in_state: false,
            })
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::PauseScheduler,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                caller,
            )
            .await?;
            Err(redis_to_ranch_err(e))
        }
    }
}

/// Resume a paused scheduler: an upsert
/// with the settings stored in its hash, which computes the next run and
/// creates the pending job again (and drops the `paused` flag: the script
/// rewrites the hash). The iteration count is kept.
///
/// (Re-adding the scheduler to `repeat` with `now` as score would create no
/// job, and only a finished job creates the next one: the scheduler would
/// never fire again.)
pub(crate) async fn handle_resume_scheduler(
    state: &mut ActorState,
    args: &SchedulerTarget,
    caller: &Caller,
    token: &CancellationToken,
) -> Result<SchedulerActionResult, RanchError> {
    let queue = QueueRef::parse(args.queue_id.as_str())?;
    authorize_write(state, caller, "resume_scheduler").await?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &queue, token).await?;

    let spec = tokio::select! {
        r = read_scheduler_spec(&mut conn, keys, &queue, &args.scheduler_key) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    let Some((spec, paused)) = spec else {
        return Err(RanchError::SchedulerNotFound {
            connection_id: state.cfg.connection_id.clone(),
            queue_name: queue.name.clone(),
            scheduler_key: args.scheduler_key.clone(),
        });
    };
    let target = AuditTarget::scheduler(&queue.id(), &queue.name, &args.scheduler_key);
    if !paused {
        write_audit_for_action(
            state,
            AuditAction::ResumeScheduler,
            target,
            AuditOutcome::Success,
            Some("already active".to_string()),
            caller,
        )
        .await?;
        return Ok(SchedulerActionResult {
            paused: false,
            already_in_state: true,
        });
    }

    let now = chrono::Utc::now();
    // Settings BullMQ wrote may use cron syntax Ranch does not author (`L`,
    // `#`…): refusing is better than re-arming at a time Ranch guessed.
    let prepared = spec
        .prepare(now)
        .map_err(|reason| RanchError::InvalidInput {
            reason: format!("Cannot resume this scheduler from Ranch: {reason}"),
        })?;
    let result = tokio::select! {
        r = run_upsert_scheduler_script(&mut conn, keys, &queue, &prepared, now.timestamp_millis()) => r,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    match result {
        Ok(Ok(_)) => {
            write_audit_for_action(
                state,
                AuditAction::ResumeScheduler,
                target,
                AuditOutcome::Success,
                None,
                caller,
            )
            .await?;
            Ok(SchedulerActionResult {
                paused: false,
                already_in_state: false,
            })
        }
        Ok(Err(refusal)) => {
            write_audit_for_action(
                state,
                AuditAction::ResumeScheduler,
                target,
                AuditOutcome::Failure,
                Some(refusal.clone()),
                caller,
            )
            .await?;
            Err(RanchError::InvalidInput { reason: refusal })
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::ResumeScheduler,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                caller,
            )
            .await?;
            Err(redis_to_ranch_err(e))
        }
    }
}
