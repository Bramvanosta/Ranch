//! One read of the queues' event streams, for the events view.

use tokio_util::sync::CancellationToken;

use super::queues::list_queues_with_refs;
use crate::bullmq_schema::Keys;
use crate::connection_actor::actor::ActorState;
use crate::connection_actor::link::{
    open_redis_for_actor, redis_to_ranch_err, resolve_keys_for_queue,
};
use ranch_domain::errors::RanchError;
use ranch_domain::events::EventsQuery;

/// The connection's queues' `…:events` streams since each
/// queue's cursor: one pipeline of `XREVRANGE + <cursor> COUNT n` (newest
/// first, capped), then `HGET name` for the jobs whose event does not carry
/// it. Pure part: `bullmq_schema::events`.
pub(crate) async fn handle_read_queue_events(
    state: &mut ActorState,
    args: &EventsQuery,
    token: &CancellationToken,
) -> Result<crate::bullmq_schema::events::QueueEvents, RanchError> {
    use crate::bullmq_schema::events::{
        after_cursor, merge, to_event, EventCursor, QueueEvents, FIRST_READ, PER_QUEUE_CAP,
    };
    let cancelled = || RanchError::Cancelled;
    let limit = args.limit.unwrap_or(100).clamp(1, 500) as usize;
    let queues = list_queues_with_refs(state, token).await?;
    if queues.is_empty() {
        return Ok(QueueEvents::default());
    }
    let mut conn = open_redis_for_actor(state, token).await?;
    let mut targets = Vec::with_capacity(queues.len());
    for (qref, q) in &queues {
        if let Ok(keys) = resolve_keys_for_queue(state, &mut conn, qref, token).await {
            let cursor = args
                .cursors
                .iter()
                .find(|c| c.queue_id.as_str() == q.id.as_str())
                .map(|c| c.last_id.clone());
            targets.push((qref.clone(), keys, cursor));
        }
    }
    let mut pipe = redis::pipe();
    for (q, keys, cursor) in &targets {
        let (from, count) = match cursor {
            Some(c) => (c.as_str(), PER_QUEUE_CAP + 1),
            None => ("-", FIRST_READ),
        };
        pipe.cmd("XREVRANGE")
            .arg(keys.events(q))
            .arg("+")
            .arg(from)
            .arg("COUNT")
            .arg(count);
    }
    let reads: Vec<Vec<crate::bullmq_schema::events::StreamEntry>> = tokio::select! {
        r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(cancelled()),
    };
    let mut events = Vec::new();
    let mut cursors = Vec::with_capacity(targets.len());
    let mut sampled = false;
    for ((q, _, cursor), entries) in targets.iter().zip(reads) {
        let (fresh, overflowed, next) = after_cursor(entries, cursor.as_deref(), PER_QUEUE_CAP + 1);
        sampled |= overflowed;
        if let Some(last_id) = next {
            cursors.push(EventCursor {
                queue_id: q.id(),
                last_id,
            });
        }
        events.extend(
            fresh
                .iter()
                .filter_map(|(id, fields)| to_event(q, id, fields)),
        );
    }
    let (mut events, cut) = merge(events, limit);
    sampled |= cut;

    // Names of the jobs whose event does not carry it (all but `added`).
    let mut wanted: Vec<usize> = Vec::new();
    let mut pipe = redis::pipe();
    for (i, e) in events.iter().enumerate() {
        let (None, Some(job_id)) = (&e.job_name, &e.job_id) else {
            continue;
        };
        if let Some((q, keys, _)) = targets.iter().find(|(q, _, _)| q.id() == e.queue_id) {
            pipe.cmd("HGET")
                .arg(keys.job_hash(&q.job(job_id.clone())))
                .arg("name");
            wanted.push(i);
        }
    }
    if !wanted.is_empty() {
        let names: Result<Vec<Option<String>>, _> = tokio::select! {
            r = pipe.query_async(&mut conn) => r,
            _ = token.cancelled() => return Err(cancelled()),
        };
        // A job removed since keeps its event, without a name.
        if let Ok(names) = names {
            for (&i, name) in wanted.iter().zip(names) {
                events[i].job_name = name;
            }
        }
    }
    Ok(QueueEvents {
        events,
        cursors,
        sampled,
    })
}
