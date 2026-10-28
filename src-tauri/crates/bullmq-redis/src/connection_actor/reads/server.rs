//! The Redis server itself (version, latency, memory, ops/s) and the
//! topology of its queues (flow links, added rates, oldest waiting jobs).

use tokio_util::sync::CancellationToken;

use super::queues::list_queues_with_refs;
use super::{read_each, read_panicked};
use crate::bullmq_schema::{KeyVariant, Keys, QueueRef};
use crate::connection_actor::actor::ActorState;
use crate::connection_actor::link::{
    open_redis_for_actor, redis_to_ranch_err, resolve_keys_for_queue,
};
use ranch_domain::errors::RanchError;

/// One timed `PING`, then `INFO` (all sections: a managed
/// Redis that refuses a section still answers the others in the default
/// reply). An `INFO` that fails leaves the fields empty, never the strip.
pub(crate) async fn handle_get_server_info(
    state: &mut ActorState,
    token: &CancellationToken,
) -> Result<crate::redis_info::ServerInfo, RanchError> {
    let cancelled = || RanchError::Cancelled;
    let mut conn = open_redis_for_actor(state, token).await?;
    let ping = redis::cmd("PING");
    let started = std::time::Instant::now();
    let _: String = tokio::select! {
        r = ping.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(cancelled()),
    };
    let latency_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
    let info_cmd = redis::cmd("INFO");
    let info: Result<String, _> = tokio::select! {
        r = info_cmd.query_async(&mut conn) => r,
        _ = token.cancelled() => return Err(cancelled()),
    };
    Ok(crate::redis_info::parse_info(
        &info.unwrap_or_default(),
        latency_ms,
    ))
}

/// The newest parents of every queue (waiting for children, or
/// finished) name their children's queues; the event streams give the added
/// rate; the tail of `wait` the oldest waiting job.
pub(crate) async fn handle_get_topology(
    state: &mut ActorState,
    token: &CancellationToken,
) -> Result<crate::bullmq_schema::topology::Topology, RanchError> {
    use crate::bullmq_schema::topology::{
        added_rate, links, queue_of_job_key, QueueFlowStats, Topology, EVENTS_PER_QUEUE,
        PARENTS_PER_QUEUE,
    };
    let now_ms = chrono::Utc::now().timestamp_millis();
    let queues = list_queues_with_refs(state, token).await?;
    let mut conn = open_redis_for_actor(state, token).await?;
    let mut readable: Vec<(QueueRef, KeyVariant)> = Vec::new();
    for (qref, _) in &queues {
        if let Ok(keys) = resolve_keys_for_queue(state, &mut conn, qref, token).await {
            readable.push((qref.clone(), keys));
        }
    }

    // Every queue's first read at once, then every second read.
    type First = (
        Vec<String>,
        Vec<String>,
        Vec<crate::bullmq_schema::events::StreamEntry>,
        Option<String>,
    );
    let firsts = read_each(
        &conn,
        readable.clone(),
        token,
        |mut conn, (qref, keys): (QueueRef, KeyVariant)| async move {
            let mut first = redis::pipe();
            first
                .cmd("ZREVRANGE")
                .arg(keys.waiting_children(&qref))
                .arg(0)
                .arg(PARENTS_PER_QUEUE as i64 - 1);
            first
                .cmd("ZREVRANGE")
                .arg(keys.completed(&qref))
                .arg(0)
                .arg(PARENTS_PER_QUEUE as i64 - 1);
            first
                .cmd("XREVRANGE")
                .arg(keys.events(&qref))
                .arg("+")
                .arg("-")
                .arg("COUNT")
                .arg(EVENTS_PER_QUEUE);
            first.cmd("LINDEX").arg(keys.wait(&qref)).arg(-1);
            first.query_async::<First>(&mut conn).await
        },
    )
    .await?;
    let firsts: Vec<First> = firsts
        .into_iter()
        .map(|r| {
            r.unwrap_or_else(|| Err(read_panicked()))
                .map_err(redis_to_ranch_err)
        })
        .collect::<Result<_, _>>()?;

    let seconds = read_each(
        &conn,
        readable
            .iter()
            .zip(&firsts)
            .map(
                |((qref, keys), (waiting_parents, done_parents, _, oldest_id))| {
                    let job = |id: &str| keys.job_hash(&qref.job(id.to_string()));
                    let mut second = redis::pipe();
                    for id in waiting_parents {
                        second
                            .cmd("SMEMBERS")
                            .arg(format!("{}:dependencies", job(id)));
                    }
                    for id in done_parents {
                        second.cmd("HKEYS").arg(format!("{}:processed", job(id)));
                    }
                    if let Some(id) = oldest_id {
                        second.cmd("HGET").arg(job(id)).arg("timestamp");
                    }
                    second
                },
            )
            .collect(),
        token,
        |mut conn, second: redis::Pipeline| async move {
            // Nothing to read (no parent, nothing waiting): an empty pipeline
            // is an error in redis-rs, not an empty reply.
            if second.cmd_iter().next().is_none() {
                return Ok(Vec::new());
            }
            second.query_async::<Vec<redis::Value>>(&mut conn).await
        },
    )
    .await?;

    let mut pairs = Vec::new();
    let mut stats = Vec::new();
    for (((qref, _), (_, _, events, oldest_id)), second) in readable.iter().zip(firsts).zip(seconds)
    {
        let mut replies = second
            .unwrap_or_else(|| Err(read_panicked()))
            .map_err(redis_to_ranch_err)?;
        let oldest_waiting_ms = if oldest_id.is_some() {
            replies
                .pop()
                .and_then(|v| redis::from_redis_value::<Option<String>>(v).ok().flatten())
                .and_then(|t| t.parse::<f64>().ok())
                .map(|t| t as i64)
        } else {
            None
        };
        for reply in replies {
            let children: Vec<String> = redis::from_redis_value(reply).unwrap_or_default();
            for child in children {
                if let Some(child_queue) = queue_of_job_key(&qref.prefix, &child) {
                    pairs.push((qref.clone(), child_queue));
                }
            }
        }
        let entries: Vec<(String, String)> = events
            .into_iter()
            .map(|(id, fields)| {
                let event = fields
                    .chunks(2)
                    .find(|kv| kv.first().is_some_and(|k| k == "event"))
                    .and_then(|kv| kv.get(1).cloned())
                    .unwrap_or_default();
                (id, event)
            })
            .collect();
        let (added_per_minute, added_capped) = added_rate(&entries, now_ms, EVENTS_PER_QUEUE);
        stats.push(QueueFlowStats {
            queue_id: qref.id(),
            added_per_minute,
            added_capped,
            oldest_waiting_ms,
        });
    }
    Ok(Topology {
        links: links(pairs),
        queues: stats,
    })
}
