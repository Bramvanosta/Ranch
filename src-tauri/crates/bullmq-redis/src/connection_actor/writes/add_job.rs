//! Enqueue a job through one atomic Lua script.

use tokio_util::sync::CancellationToken;

use super::authorize::{authorize_write, write_audit_for_action};
use crate::bullmq_schema::add_job::{self, PreparedJob};
use crate::bullmq_schema::states;
use crate::bullmq_schema::QueueRef;
use crate::bullmq_schema::{JobState, Keys, V5Keys};
use crate::connection_actor::actor::ActorState;
use crate::connection_actor::link::{
    open_redis_for_actor, redis_to_ranch_err, resolve_keys_for_queue,
};
use ranch_domain::audit::{AuditAction, AuditOutcome, AuditTarget, Caller};
use ranch_domain::errors::RanchError;
use ranch_domain::jobs::{AddJobResult, NewJob};
use ranch_domain::queues::QueueId;

/// Enqueue a job.
///
///   1. safety gate (MCP approval / session lock), before anything else;
///   2. validate the input (pure, `NewJob::prepare`): not audited, nothing
///      was attempted;
///   3. the queue must exist (`meta`): neither the UI nor an agent may
///      conjure a queue out of a typo;
///   4. one Lua script does what BullMQ's add-job scripts do (id, hash,
///      placement by delay / priority / paused, marker, events), atomically;
///   5. audit, success or failure.
pub(crate) async fn handle_add_job(
    state: &mut ActorState,
    queue_id: &QueueId,
    job: &NewJob,
    caller: &Caller,
    token: &CancellationToken,
) -> Result<AddJobResult, RanchError> {
    authorize_write(state, caller, "add_job").await?;

    let job = PreparedJob::try_from(job).map_err(|reason| RanchError::InvalidInput { reason })?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let q = &QueueRef::parse(queue_id.as_str())?;

    // Before the schema: on a cold cache a missing queue would otherwise
    // read as "schema not recognized". `:meta` is the same key in every
    // supported version.
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

    let script = redis::Script::new(add_job::ADD_JOB_LUA);
    let mut invocation = script.prepare_invoke();
    invocation
        .key(keys.wait(q))
        .key(keys.paused(q))
        .key(keys.meta(q))
        .key(keys.id_counter(q))
        .key(keys.delayed(q))
        .key(keys.prioritized(q))
        .key(keys.active(q))
        .key(keys.events(q))
        .key(keys.marker(q))
        .key(keys.priority_counter(q))
        .arg(format!("{}:", keys.queue_base(q)))
        .arg(&job.name)
        .arg(&job.data)
        .arg(&job.opts)
        .arg(chrono::Utc::now().timestamp_millis())
        .arg(job.delay_ms)
        .arg(job.priority)
        .arg(if job.lifo { "1" } else { "0" })
        .arg(job.job_id.as_deref().unwrap_or(""));
    let result: Result<(String, String), redis::RedisError> = tokio::select! {
        r = invocation.invoke_async(&mut conn) => r,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };

    match result {
        // A custom id already in use: BullMQ would ignore the
        // add; Ranch says so instead of pretending.
        Ok((job_id, placement)) if placement == "exists" => Err(RanchError::InvalidInput {
            reason: format!(
                "A job with ID \"{job_id}\" already exists in {}: nothing was added",
                q.name
            ),
        }),
        Ok((job_id, placement)) => {
            write_audit_for_action(
                state,
                AuditAction::AddJob,
                AuditTarget::job(&q.id(), &q.name, &job_id),
                AuditOutcome::Success,
                Some(format!("{} → {placement}", job.name)),
                caller,
            )
            .await?;
            let state = match placement.as_str() {
                "delayed" => JobState::Delayed,
                "prioritized" => states::PRIORITIZED,
                "paused" => JobState::Paused,
                _ => JobState::Waiting,
            };
            Ok(AddJobResult { job_id, state })
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::AddJob,
                AuditTarget::queue(&q.id(), &q.name),
                AuditOutcome::Failure,
                Some(e.to_string()),
                caller,
            )
            .await?;
            Err(redis_to_ranch_err(e))
        }
    }
}
