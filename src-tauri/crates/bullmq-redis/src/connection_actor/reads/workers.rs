//! BullMQ workers from `CLIENT LIST`: per queue, and counted for many
//! queues from one reply. A Redis that refuses the command is remembered.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::bullmq_schema::workers;
use crate::bullmq_schema::QueueRef;
use crate::bullmq_schema::{QueueWorkerCounts, QueueWorkers};
use crate::connection_actor::actor::{lock, ActorState};
use crate::connection_actor::link::{open_redis_for_actor, redis_to_ranch_err};
use ranch_domain::errors::RanchError;
use ranch_domain::queues::QueueId;

/// The BullMQ workers attached to a queue. BullMQ keeps
/// no registry: a worker is a Redis connection named after the queue, so
/// this is `CLIENT LIST` + the pure matcher in `bullmq_schema::workers`.
///
/// A provider that forbids `CLIENT` is a capability, not an error: the
/// refusal is cached on the actor and reported as `supported: false`.
/// Network / auth failures stay errors and are not cached.
pub(crate) async fn handle_list_workers(
    state: &mut ActorState,
    queue_id: &QueueId,
    asked_at: std::time::Instant,
    token: &CancellationToken,
) -> Result<QueueWorkers, RanchError> {
    let queue = QueueRef::parse(queue_id.as_str())?;
    match client_list(state, asked_at, token).await? {
        Ok(raw) => Ok(QueueWorkers {
            supported: true,
            unsupported_reason: None,
            workers: workers::workers_for_queue(&raw, &queue),
        }),
        Err(reason) => Ok(QueueWorkers::unsupported(reason)),
    }
}

/// Worker counts of many queues from one `CLIENT LIST`; a
/// Redis that refuses the command answers `supported: false`.
pub(crate) async fn handle_count_queue_workers(
    state: &mut ActorState,
    queue_ids: &[QueueId],
    asked_at: std::time::Instant,
    token: &CancellationToken,
) -> Result<QueueWorkerCounts, RanchError> {
    let queues = queue_ids
        .iter()
        .map(|id| QueueRef::parse(id.as_str()))
        .collect::<Result<Vec<_>, _>>()?;
    match client_list(state, asked_at, token).await? {
        Ok(raw) => Ok(QueueWorkerCounts {
            supported: true,
            counts: workers::count_workers(&raw, &queues),
        }),
        Err(_) => Ok(QueueWorkerCounts {
            supported: false,
            counts: Vec::new(),
        }),
    }
}

/// The raw `CLIENT LIST` reply, or `Err(reason)` when this Redis refuses the
/// command (cached on the actor: a provider that forbids it is not asked
/// again). Network / auth failures stay errors and are not cached.
async fn client_list(
    state: &mut ActorState,
    asked_at: std::time::Instant,
    token: &CancellationToken,
) -> Result<Result<String, String>, RanchError> {
    let denied = lock(&state.client_list_denied).clone();
    if let Some(reason) = denied {
        return Ok(Err(reason));
    }
    // A reply that arrived after this call was made is as fresh as reading
    // again: the calls of one burst share one read, and a later call always
    // reads. `asked_at` is when the actor received the call, before it
    // waited for a read slot, so a burst wider than the slots still shares.
    let cache = Arc::clone(&state.client_list_cache);
    let mut cached = cache.lock().await;
    if let Some((arrived_at, raw)) = cached.as_ref() {
        if *arrived_at >= asked_at {
            return Ok(Ok(raw.clone()));
        }
    }
    let mut conn = open_redis_for_actor(state, token).await?;
    let mut cmd = redis::cmd("CLIENT");
    cmd.arg("LIST");
    let reply: Result<String, redis::RedisError> = tokio::select! {
        r = cmd.query_async::<String>(&mut conn) => r,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    match reply {
        Ok(raw) => {
            *cached = Some((std::time::Instant::now(), raw.clone()));
            Ok(Ok(raw))
        }
        Err(e) if workers::client_list_denied(&e.to_string()) => {
            let reason = e.to_string();
            tracing::info!(
                connection = %state.cfg.connection_name,
                %reason,
                "CLIENT LIST refused by this Redis; workers view disabled for the connection"
            );
            *lock(&state.client_list_denied) = Some(reason.clone());
            Ok(Err(reason))
        }
        Err(e) => Err(redis_to_ranch_err(e)),
    }
}
