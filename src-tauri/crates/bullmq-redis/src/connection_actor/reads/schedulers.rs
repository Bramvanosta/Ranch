//! The job schedulers (CRON / repeatable) of a queue.

use std::collections::HashMap;

use tokio_util::sync::CancellationToken;

use super::redis_value_hash_to_bytes;
use crate::bullmq_schema::QueueRef;
use crate::bullmq_schema::{decode_cron_hash, CronJob, Keys};
use crate::connection_actor::actor::ActorState;
use crate::connection_actor::link::{
    open_redis_for_actor, redis_to_ranch_err, resolve_keys_for_queue,
};
use ranch_domain::errors::RanchError;
use ranch_domain::queues::QueueId;

/// List CRON / repeatable schedulers for a queue.
/// Reads the `bull:{queue}:repeat` ZSET + pipelines HGETALL per
/// scheduler key. Per-scheduler resilience: empty hash or invalid
/// decode → `tracing::warn!` + skip (don't fail the whole call).
pub(crate) async fn handle_list_cron_jobs(
    state: &mut ActorState,
    queue_id: &QueueId,
    token: &CancellationToken,
) -> Result<Vec<CronJob>, RanchError> {
    let domain_queue = QueueRef::parse(queue_id.as_str())?;
    // The SSH tunnel, when there is one, is opened inside `open_redis_for_actor`.
    let mut conn = open_redis_for_actor(state, token).await?;

    let keys = resolve_keys_for_queue(state, &mut conn, &domain_queue, token).await?;
    let repeat_key = keys.repeat(&domain_queue);

    // ZRANGE WITHSCORES: returns Vec<(member, score)>.
    let mut zrange_cmd = redis::cmd("ZRANGE");
    zrange_cmd
        .arg(&repeat_key)
        .arg(0_i64)
        .arg(-1_i64)
        .arg("WITHSCORES");
    let members_with_scores: Vec<(String, f64)> = tokio::select! {
        r = zrange_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::Cancelled);
        }
    };

    if members_with_scores.is_empty() {
        return Ok(Vec::new());
    }

    // Pipeline HGETALL per scheduler key, non-atomic (independent reads).
    let hash_keys: Vec<String> = members_with_scores
        .iter()
        .map(|(member, _)| format!("{repeat_key}:{member}"))
        .collect();

    let mut pipe = redis::pipe();
    for hk in &hash_keys {
        pipe.cmd("HGETALL").arg(hk);
    }
    let raw_hashes: Vec<HashMap<String, redis::Value>> = tokio::select! {
        r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::Cancelled);
        }
    };

    let mut out: Vec<CronJob> = Vec::with_capacity(members_with_scores.len());
    for ((member, score), raw) in members_with_scores.into_iter().zip(raw_hashes) {
        if raw.is_empty() {
            tracing::warn!(
                queue = %queue_id,
                scheduler_key = %member,
                "scheduler hash empty (removed mid-flight); skipping"
            );
            continue;
        }
        let fields = redis_value_hash_to_bytes(raw);
        // BullMQ writes scheduler scores as ms-since-epoch integers; redis-rs
        // gives them back as f64 from a WITHSCORES query. Cast safely.
        let next_run_ms = score as i64;
        if let Some(cj) = decode_cron_hash(member, next_run_ms, &fields) {
            out.push(cj);
        }
        // `decode_cron_hash` already emits a `tracing::warn!` on skip cases
        // (missing name, both/neither pattern+every).
    }

    Ok(out)
}
