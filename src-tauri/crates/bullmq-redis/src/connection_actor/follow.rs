//! Event following: while a window follows the connection, one task reads
//! the queues' event streams (`XREAD BLOCK`) on a connection of its own
//! and tells the [`EventSink`] which queues moved.

use std::collections::HashMap;

use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::actor::{lock, ActorState};
use super::link::{
    build_redis_url_from_config, build_redis_url_via_local, ensure_ssh_tunnel, redis_to_ranch_err,
};
use crate::bullmq_schema::{Keys, QueueRef, V5Keys};
use crate::tls::{build_redis_client, validate_ca_pem};
use ranch_domain::errors::RanchError;

/// Entries read per XREAD, all streams together (capped).
const FOLLOW_COUNT: usize = 500;

/// How long one XREAD waits for news before asking again.
const FOLLOW_BLOCK_MS: usize = 5_000;

/// At most one `queue:events` per this: a busy Redis writes
/// events all the time, the app refreshes in batches.
const FOLLOW_MIN_GAP: std::time::Duration = std::time::Duration::from_millis(300);

/// A connection of its own for the blocking reads (a blocked
/// XREAD would hold every request queued behind it on the shared one), with
/// no reply timeout: the server answers when it has news or after
/// `FOLLOW_BLOCK_MS`.
async fn open_follower_connection(
    state: &ActorState,
) -> Result<redis::aio::MultiplexedConnection, RanchError> {
    let validated_ca = validate_ca_pem(state.cfg.endpoint.tls_custom_ca.as_deref())?;
    let (url, ca) = {
        let mut link = state.link.lock().await;
        if state.cfg.endpoint.ssh_tunnel.is_some() {
            let local = ensure_ssh_tunnel(&state.cfg, &mut link).await?;
            (build_redis_url_via_local(&state.cfg, local), None)
        } else {
            (build_redis_url_from_config(&state.cfg), validated_ca)
        }
    };
    let client = build_redis_client(&url, ca.as_deref()).map_err(redis_to_ranch_err)?;
    let config = redis::AsyncConnectionConfig::new()
        .set_response_timeout(None)
        .set_connection_timeout(Some(crate::tls::REDIS_CONNECT_TIMEOUT));
    client
        .get_multiplexed_async_connection_with_config(&config)
        .await
        .map_err(redis_to_ranch_err)
}

type StreamRead = Option<Vec<(String, Vec<(String, Vec<String>)>)>>;

pub use ranch_application::ports::backend::EventSink;

/// Read the queues' event streams (`XREAD BLOCK`; BullMQ
/// writes them for its own `QueueEvents`, so no Redis setting is needed)
/// and tell the windows which queues moved (`queue:events`), so they refresh
/// those instead of polling everything. Runs while a window follows the
/// connection (the visible one, while shown); the queues are the
/// registry's, new ones join on the next read.
pub(super) async fn follow_events(
    state: ActorState,
    sink: EventSink,
    token: CancellationToken,
    mut ready: Option<oneshot::Sender<bool>>,
) {
    use ranch_domain::events::{QueueEventsNotice, QueueEventsSeen};
    let pause = |ms: u64| tokio::time::sleep(std::time::Duration::from_millis(ms));
    let mut conn: Option<redis::aio::MultiplexedConnection> = None;
    // Where each stream was read up to. A stream read for the first time
    // starts at the Redis clock when following began: not `$`, which would
    // drop what arrives between two reads, nor the time of that first read,
    // which would drop what arrived before it. XREAD returns what comes
    // after the id it is given, so that id is the last one before the start.
    let mut cursors: HashMap<String, String> = HashMap::new();
    let mut start_ms: Option<i64> = None;
    let mut last_emit = std::time::Instant::now() - FOLLOW_MIN_GAP;
    loop {
        if token.is_cancelled() {
            return;
        }
        if conn.is_none() {
            match open_follower_connection(&state).await {
                Ok(c) => conn = Some(c),
                Err(e) => {
                    tracing::warn!(error = %e, "event follower could not connect; retrying");
                    if let Some(ready) = ready.take() {
                        let _ = ready.send(true);
                    }
                    tokio::select! { _ = pause(2_000) => continue, _ = token.cancelled() => return }
                }
            }
        }
        let Some(c) = conn.as_mut() else { continue };
        if start_ms.is_none() {
            let time: Result<(String, String), _> = redis::cmd("TIME").query_async(c).await;
            let server_ms = time
                .ok()
                .and_then(|(s, us)| {
                    Some(s.parse::<i64>().ok()? * 1_000 + us.parse::<i64>().ok()? / 1_000)
                })
                .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
            start_ms = Some(server_ms);
            if let Some(ready) = ready.take() {
                let _ = ready.send(true);
            }
        }
        let queues: Vec<QueueRef> = lock(&state.queue_registry)
            .current
            .as_ref()
            .map(|r| r.queues.iter().map(|(q, _)| q.clone()).collect())
            .unwrap_or_default();
        if queues.is_empty() {
            tokio::select! { _ = pause(1_000) => continue, _ = token.cancelled() => return }
        }
        let before_start = match start_ms.unwrap_or(0) {
            0 => "0-0".to_string(),
            ms => format!("{}-{}", ms - 1, u64::MAX),
        };
        let keys: Vec<(String, QueueRef)> =
            queues.into_iter().map(|q| (V5Keys.events(&q), q)).collect();
        let mut cmd = redis::cmd("XREAD");
        cmd.arg("COUNT")
            .arg(FOLLOW_COUNT)
            .arg("BLOCK")
            .arg(FOLLOW_BLOCK_MS)
            .arg("STREAMS");
        for (key, _) in &keys {
            cmd.arg(key);
        }
        for (key, _) in &keys {
            match cursors.get(key) {
                Some(id) => cmd.arg(id),
                None => cmd.arg(&before_start),
            };
        }
        let read: Result<StreamRead, redis::RedisError> = tokio::select! {
            r = cmd.query_async(c) => r,
            _ = token.cancelled() => return,
        };
        let streams = match read {
            Ok(Some(streams)) => streams,
            Ok(None) => continue, // nothing new within the block
            Err(e) => {
                tracing::warn!(error = %e, "event follower read failed; reconnecting");
                conn = None;
                tokio::select! { _ = pause(2_000) => continue, _ = token.cancelled() => return }
            }
        };
        let mut seen: Vec<QueueEventsSeen> = Vec::new();
        for (key, entries) in streams {
            let Some((last_id, _)) = entries.last() else {
                continue;
            };
            cursors.insert(key.clone(), last_id.clone());
            let Some((_, queue_ref)) = keys.iter().find(|(k, _)| *k == key) else {
                continue;
            };
            let mut events: Vec<String> = Vec::new();
            for (_, fields) in &entries {
                let name = fields
                    .chunks(2)
                    .find(|kv| kv.first().is_some_and(|k| k == "event"))
                    .and_then(|kv| kv.get(1).cloned());
                if let Some(name) = name {
                    if !events.contains(&name) {
                        events.push(name);
                    }
                }
            }
            seen.push(QueueEventsSeen {
                queue_id: queue_ref.id(),
                events,
                count: entries.len() as u32,
            });
        }
        if seen.is_empty() {
            continue;
        }
        // Batch: at most one notice per FOLLOW_MIN_GAP.
        let since = last_emit.elapsed();
        if since < FOLLOW_MIN_GAP {
            tokio::select! { _ = tokio::time::sleep(FOLLOW_MIN_GAP - since) => {}, _ = token.cancelled() => return }
        }
        sink(QueueEventsNotice {
            connection_id: state.cfg.connection_id.clone(),
            queues: seen,
        });
        last_emit = std::time::Instant::now();
    }
}
