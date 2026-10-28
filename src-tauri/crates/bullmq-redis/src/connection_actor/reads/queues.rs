//! The queue list: the registry's queues with their counters and pause
//! flag, read in one pipeline.

use tokio_util::sync::CancellationToken;

use crate::bullmq_schema::{
    states, BullmqSchema, JobState, KeyVariant, Keys, QueueRef, V5Keys, V6Keys,
};
use crate::connection_actor::actor::{lock, ActorState};
use crate::connection_actor::discovery::{pick_or_cache_schema, queue_registry, REGISTRY_TTL};
use crate::connection_actor::link::{open_redis_for_actor, redis_to_ranch_err};
use ranch_domain::errors::RanchError;
use ranch_domain::queues::Queue;

/// List the queues and their counters.
///
/// The queues come from the actor's registry (a SCAN, cached
/// and refreshed in the background, see [`queue_registry`]); every queue's
/// counters and pause flag are then read in **one pipeline**: one round trip
/// whatever the number of queues. A failed read is an error, never
/// zeros.
///
/// Returns `Vec<Queue>` sorted by name (case-insensitive).
/// Empty list when no queues are found. Schema-not-recognized when the
/// only discovered queues use unsupported BullMQ versions.
pub(crate) async fn handle_list_queues(
    state: &mut ActorState,
    token: &CancellationToken,
) -> Result<Vec<Queue>, RanchError> {
    Ok(list_queues_with_refs(state, token)
        .await?
        .into_iter()
        .map(|(_, queue)| queue)
        .collect())
}

/// [`handle_list_queues`], each queue with the BullMQ queue it is, for the
/// reads that go on to its keys.
pub(crate) async fn list_queues_with_refs(
    state: &mut ActorState,
    token: &CancellationToken,
) -> Result<Vec<(QueueRef, Queue)>, RanchError> {
    let mut conn = open_redis_for_actor(state, token).await?;
    let detected = queue_registry(state, &conn, token).await?;
    if detected.is_empty() {
        return Ok(Vec::new());
    }

    // Cache the schema (first-success-wins). If every detected schema is
    // Unknown, surface a typed schema-not-recognized error.
    let schema = pick_or_cache_schema(&mut lock(&state.cached_schema), &detected)?;
    let keys: KeyVariant = match schema {
        BullmqSchema::V5 => KeyVariant::V5(V5Keys),
        BullmqSchema::V6 => KeyVariant::V6(V6Keys),
        // pick_or_cache_schema returns Err for Unknown.
        BullmqSchema::Unknown { .. } => unreachable!("pick_or_cache_schema rejects Unknown"),
    };

    let mut pipe = redis::pipe();
    for (qref, _) in &detected {
        push_counter_reads(&mut pipe, keys, qref);
    }
    let replies: Vec<redis::Value> = tokio::select! {
        r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::Cancelled);
        }
    };

    let mut queues: Vec<(QueueRef, Queue)> = Vec::with_capacity(detected.len());
    let mut gone = false;
    for ((qref, _), reply) in detected.iter().zip(replies.chunks(COUNTER_READS)) {
        match counters_from(qref, reply).map_err(redis_to_ranch_err)? {
            Some(queue) => queues.push((qref.clone(), queue)),
            // Deleted since the SCAN (obliterated): left out, and the
            // registry is refreshed on the next call.
            None => gone = true,
        }
    }
    if gone {
        if let Ok(mut slot) = state.queue_registry.lock() {
            if let Some(current) = slot.current.as_mut() {
                current.scanned_at -= REGISTRY_TTL;
            }
        }
    }

    // Sort by queue name (case-insensitive) for stable polling output.
    queues.sort_by_key(|(_, q)| q.name.to_lowercase());

    Ok(queues)
}

/// Replies per queue in [`handle_list_queues`]'s pipeline.
const COUNTER_READS: usize = 9;

/// A queue's counters, pause flag and existence, appended to a pipeline
/// ([`COUNTER_READS`] replies, read by [`counters_from`]).
///
/// "is it paused?" is BullMQ's `meta.paused` flag, not the
/// existence of the `paused` list: that list only exists when jobs were
/// waiting at pause time, so an empty paused queue read as running.
fn push_counter_reads(pipe: &mut redis::Pipeline, keys: KeyVariant, qref: &QueueRef) {
    pipe.cmd("LLEN")
        .arg(keys.wait(qref))
        .cmd("LLEN")
        .arg(keys.active(qref))
        .cmd("ZCARD")
        .arg(keys.completed(qref))
        .cmd("ZCARD")
        .arg(keys.failed(qref))
        .cmd("ZCARD")
        .arg(keys.delayed(qref))
        .cmd("ZCARD")
        .arg(keys.prioritized(qref))
        .cmd("LLEN")
        .arg(keys.paused(qref))
        .cmd("HEXISTS")
        .arg(keys.meta(qref))
        .arg("paused")
        .cmd("EXISTS")
        .arg(keys.meta(qref));
}

/// Read [`push_counter_reads`]'s replies. `None` when the queue's meta is
/// gone (the queue was obliterated since the registry's SCAN).
fn counters_from(qref: &QueueRef, r: &[redis::Value]) -> Result<Option<Queue>, redis::RedisError> {
    let n = |i: usize| -> Result<u64, redis::RedisError> {
        Ok(redis::from_redis_value(
            r.get(i).cloned().unwrap_or(redis::Value::Nil),
        )?)
    };
    let flag = |i: usize| -> Result<bool, redis::RedisError> {
        Ok(redis::from_redis_value(
            r.get(i).cloned().unwrap_or(redis::Value::Nil),
        )?)
    };
    if r.len() < COUNTER_READS || !flag(8)? {
        return Ok(None);
    }
    Ok(Some(Queue {
        id: qref.id(),
        name: qref.name.clone(),
        counts: [
            (JobState::Waiting, n(0)?),
            (JobState::Active, n(1)?),
            (JobState::Completed, n(2)?),
            (JobState::Failed, n(3)?),
            (JobState::Delayed, n(4)?),
            (states::PRIORITIZED, n(5)?),
            (JobState::Paused, n(6)?),
        ]
        .into_iter()
        .collect(),
        is_paused: flag(7)?,
    }))
}
