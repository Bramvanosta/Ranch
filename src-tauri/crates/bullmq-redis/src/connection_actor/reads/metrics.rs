//! Metrics: a queue's 24 h dashboard, the throughput of every queue, and
//! the summary of a Timeline window.

use tokio_util::sync::CancellationToken;

use super::queues::list_queues_with_refs;
use super::{read_each, read_panicked};
use crate::bullmq_schema::{KeyVariant, Keys, QueueRef};
use crate::connection_actor::actor::ActorState;
use crate::connection_actor::link::{
    open_redis_for_actor, redis_to_ranch_err, resolve_keys_for_queue,
};
use ranch_domain::errors::RanchError;
use ranch_domain::metrics::{MetricsRange, QueueMetrics, QueueMetricsQuery, Throughput};
use ranch_domain::timeline::{WindowQuery, WindowSummary};

/// Completed / failed over time for every queue of the
/// connection, from native metrics only: one pipeline of `HMGET` + `LRANGE`
/// per queue and kind. A queue whose keys cannot be resolved is left out.
pub(crate) async fn handle_get_throughput(
    state: &mut ActorState,
    range: MetricsRange,
    token: &CancellationToken,
) -> Result<Throughput, RanchError> {
    use ranch_domain::metrics::{throughput, NativeMetrics};
    let cancelled = || RanchError::Cancelled;
    let queues = list_queues_with_refs(state, token).await?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    if queues.is_empty() {
        return Ok(throughput(range, now_ms, Vec::new()));
    }
    let mut conn = open_redis_for_actor(state, token).await?;
    let mut resolved = Vec::with_capacity(queues.len());
    for (qref, _) in &queues {
        if let Ok(keys) = resolve_keys_for_queue(state, &mut conn, qref, token).await {
            resolved.push((qref.clone(), keys));
        }
    }
    let minutes = range.window_minutes();
    let mut pipe = redis::pipe();
    for (q, keys) in &resolved {
        for kind in ["completed", "failed"] {
            pipe.cmd("HMGET")
                .arg(keys.metrics_meta(q, kind))
                .arg("count")
                .arg("prevCount")
                .arg("prevTS");
            pipe.cmd("LRANGE")
                .arg(keys.metrics_data(q, kind))
                .arg(0_i64)
                .arg(minutes);
        }
    }
    let raw: Vec<redis::Value> = tokio::select! {
        r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(cancelled()),
    };
    let mut it = raw.into_iter();
    let mut native = || -> NativeMetrics {
        let meta: Vec<Option<String>> =
            redis::from_redis_value(it.next().unwrap_or(redis::Value::Nil)).unwrap_or_default();
        let data: Vec<String> =
            redis::from_redis_value(it.next().unwrap_or(redis::Value::Nil)).unwrap_or_default();
        let num = |i: usize| -> i64 {
            meta.get(i)
                .and_then(|v| v.as_deref())
                .and_then(|s| s.parse::<f64>().ok())
                .map(|v| v as i64)
                .unwrap_or(0)
        };
        NativeMetrics {
            count: num(0).max(0) as u64,
            prev_count: num(1).max(0) as u64,
            prev_ts_ms: num(2),
            data: data
                .iter()
                .map(|s| s.parse::<f64>().map(|v| v.max(0.0) as u64).unwrap_or(0))
                .collect(),
        }
    };
    let mut per_queue = Vec::with_capacity(resolved.len());
    for (q, _) in resolved {
        let completed = native();
        let failed = native();
        per_queue.push((q.id(), q.name, completed, failed));
    }
    Ok(throughput(range, now_ms, per_queue))
}

/// The jobs that finished in the visible range (the completed
/// and failed sets are scored by finish time), newest first and capped per
/// queue and state, summarized for the dragged window.
pub(crate) async fn handle_summarize_window(
    state: &mut ActorState,
    args: &WindowQuery,
    token: &CancellationToken,
) -> Result<WindowSummary, RanchError> {
    use crate::bullmq_schema::timeline::{summarize, FinishedJob, WINDOW_SCAN_PER_SET};
    let now_ms = chrono::Utc::now().timestamp_millis();
    let queues = list_queues_with_refs(state, token).await?;
    let mut conn = open_redis_for_actor(state, token).await?;

    // Every (queue, completed|failed) set read at once: its
    // finished ids in the range, then their timings.
    let mut sets: Vec<(QueueRef, KeyVariant, bool)> = Vec::new();
    for (qref, _) in &queues {
        if let Ok(keys) = resolve_keys_for_queue(state, &mut conn, qref, token).await {
            sets.push((qref.clone(), keys, false));
            sets.push((qref.clone(), keys, true));
        }
    }
    let range_from_ms = args.range_from_ms;
    type Rows = (Vec<String>, Vec<Vec<Option<String>>>);
    let reads = read_each(
        &conn,
        sets.clone(),
        token,
        move |mut conn, (qref, keys, failed): (QueueRef, KeyVariant, bool)| async move {
            let set = if failed {
                keys.failed(&qref)
            } else {
                keys.completed(&qref)
            };
            let ids: Vec<String> = redis::cmd("ZREVRANGEBYSCORE")
                .arg(&set)
                .arg(now_ms)
                .arg(range_from_ms)
                .arg("LIMIT")
                .arg(0)
                .arg(WINDOW_SCAN_PER_SET)
                .query_async(&mut conn)
                .await?;
            if ids.is_empty() {
                return Ok::<Rows, redis::RedisError>((ids, Vec::new()));
            }
            let mut pipe = redis::pipe();
            for id in &ids {
                pipe.cmd("HMGET")
                    .arg(keys.job_hash(&qref.job(id.clone())))
                    .arg("timestamp")
                    .arg("processedOn")
                    .arg("finishedOn")
                    .arg("failedReason");
            }
            let rows = pipe.query_async(&mut conn).await?;
            Ok((ids, rows))
        },
    )
    .await?;

    let mut jobs: Vec<FinishedJob> = Vec::new();
    let mut capped = false;
    for ((qref, _, failed), read) in sets.into_iter().zip(reads) {
        let (ids, rows) = read
            .unwrap_or_else(|| Err(read_panicked()))
            .map_err(redis_to_ranch_err)?;
        capped |= ids.len() >= WINDOW_SCAN_PER_SET;
        for (id, row) in ids.into_iter().zip(rows) {
            let num = |i: usize| -> Option<i64> {
                row.get(i)
                    .and_then(|v| v.as_deref())
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|v| v as i64)
            };
            let Some(finished_on) = num(2) else {
                continue; // removed under our feet
            };
            jobs.push(FinishedJob {
                job_ref: qref.job(id),
                failed,
                timestamp: num(0).unwrap_or(finished_on),
                processed_on: num(1),
                finished_on,
                failed_reason: row.get(3).cloned().flatten(),
            });
        }
    }
    Ok(summarize(
        &jobs,
        args.from_ms,
        args.to_ms,
        args.range_from_ms,
        now_ms,
        capped,
    ))
}

/// Metrics of one queue over a selectable window.
/// All arithmetic lives in `domain::metrics`; this
/// handler only fetches, in three pipelined round-trips:
///
///   1. native metrics (`metrics:completed|failed` hash + `:data` list) AND
///      the `ZCOUNT`-per-bucket estimate, in one pipeline;
///   2. the ids of the most recent completed / failed jobs in the window
///      (`ZREVRANGEBYSCORE … LIMIT 0 N`);
///   3. `HMGET` of the fields metrics need, for those ids.
///
/// Counts come from native metrics when the queue records them (exact, and
/// immune to `removeOnComplete`), from the estimate otherwise. Timing
/// figures, slowest jobs and failing types always come from the sample.
pub(crate) async fn handle_get_queue_metrics(
    state: &mut ActorState,
    args: &QueueMetricsQuery,
    token: &CancellationToken,
) -> Result<QueueMetrics, RanchError> {
    let domain_queue = QueueRef::parse(args.queue_id.as_str())?;
    use ranch_domain::metrics::{build, Buckets, JobSample, NativeMetrics, QueueMetricsInput};
    /// Bounded sample per state: enough for stable averages and a top 10,
    /// small enough for one HMGET pipeline.
    const SAMPLE: i64 = 200;

    let cancelled = || RanchError::Cancelled;

    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &domain_queue, token).await?;
    let q = &domain_queue;
    let completed_key = keys.completed(q);
    let failed_key = keys.failed(q);

    let now_ms = args
        .now_ms
        .map(|v| v as i64)
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let range = args.range;
    let buckets = Buckets::new(range, now_ms);
    let n = buckets.count();
    let minutes = range.window_minutes();

    // ── 1. native metrics + estimate, one round-trip ─────────────────────
    let mut pipe = redis::pipe();
    for kind in ["completed", "failed"] {
        pipe.cmd("HMGET")
            .arg(keys.metrics_meta(q, kind))
            .arg("count")
            .arg("prevCount")
            .arg("prevTS");
        pipe.cmd("LRANGE")
            .arg(keys.metrics_data(q, kind))
            .arg(0_i64)
            .arg(minutes);
    }
    for key in [&completed_key, &failed_key] {
        for i in 0..n {
            let (lo, hi) = buckets.zcount_bounds(i);
            pipe.cmd("ZCOUNT").arg(key).arg(lo).arg(hi);
        }
    }
    type NativeRaw = (Vec<Option<String>>, Vec<String>);
    let (native_completed_raw, native_failed_raw, counts): (NativeRaw, NativeRaw, Vec<u64>) = {
        // The pipeline returns 4 + 2n values; read them positionally.
        let raw: Vec<redis::Value> = tokio::select! {
            r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => return Err(cancelled()),
        };
        let mut it = raw.into_iter();
        let mut take = || it.next().unwrap_or(redis::Value::Nil);
        let decode_native = |meta: redis::Value, data: redis::Value| -> NativeRaw {
            (
                redis::from_redis_value(meta).unwrap_or_default(),
                redis::from_redis_value(data).unwrap_or_default(),
            )
        };
        let completed = decode_native(take(), take());
        let failed = decode_native(take(), take());
        let counts: Vec<u64> = it
            .map(|v| redis::from_redis_value::<u64>(v).unwrap_or(0))
            .collect();
        (completed, failed, counts)
    };
    let to_native = |(meta, data): NativeRaw| -> NativeMetrics {
        let num = |i: usize| -> Option<i64> {
            meta.get(i)
                .and_then(|v| v.as_deref())
                .and_then(|s| s.parse::<f64>().ok())
                .map(|v| v as i64)
        };
        NativeMetrics {
            count: num(0).unwrap_or(0).max(0) as u64,
            prev_count: num(1).unwrap_or(0).max(0) as u64,
            prev_ts_ms: num(2).unwrap_or(0),
            data: data
                .iter()
                .map(|s| s.parse::<f64>().map(|v| v.max(0.0) as u64).unwrap_or(0))
                .collect(),
        }
    };
    let native_completed = to_native(native_completed_raw);
    let native_failed = to_native(native_failed_raw);
    let estimated_completed: Vec<u64> = counts.iter().take(n).copied().collect();
    let estimated_failed: Vec<u64> = counts.iter().skip(n).take(n).copied().collect();

    // ── 2. most recent finished job ids in the window ─────────────────────
    let mut pipe = redis::pipe();
    for key in [&completed_key, &failed_key] {
        pipe.cmd("ZREVRANGEBYSCORE")
            .arg(key)
            .arg(now_ms)
            .arg(buckets.window_start_ms())
            .arg("LIMIT")
            .arg(0_i64)
            .arg(SAMPLE);
    }
    let (completed_ids, failed_ids): (Vec<String>, Vec<String>) = tokio::select! {
        r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(cancelled()),
    };

    // ── 3. the fields metrics need, for those jobs ───────────────────────
    let all_ids: Vec<&String> = completed_ids.iter().chain(failed_ids.iter()).collect();
    let mut samples: Vec<JobSample> = Vec::with_capacity(all_ids.len());
    if !all_ids.is_empty() {
        let mut pipe = redis::pipe();
        for id in &all_ids {
            pipe.cmd("HMGET")
                .arg(keys.job_hash(&q.job((*id).clone())))
                .arg("name")
                .arg("timestamp")
                .arg("processedOn")
                .arg("finishedOn")
                .arg("delay")
                .arg("failedReason");
        }
        let rows: Vec<Vec<Option<String>>> = tokio::select! {
            r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => return Err(cancelled()),
        };
        for (id, fields) in all_ids.iter().zip(rows) {
            let text = |i: usize| fields.get(i).and_then(|v| v.clone());
            let int = |i: usize| {
                text(i)
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|v| v as i64)
            };
            samples.push(JobSample {
                job_id: (*id).clone(),
                name: text(0).unwrap_or_default(),
                timestamp: int(1),
                processed_on: int(2),
                finished_on: int(3),
                delay: int(4),
                failed_reason: text(5).filter(|s| !s.is_empty()),
            });
        }
    }
    let (completed_sample, failed_sample) = samples.split_at(completed_ids.len());

    Ok(build(QueueMetricsInput {
        queue_id: &q.id(),
        queue_name: &q.name,
        buckets: &buckets,
        native_completed: Some(&native_completed),
        native_failed: Some(&native_failed),
        estimated_completed: &estimated_completed,
        estimated_failed: &estimated_failed,
        completed_sample,
        failed_sample,
    }))
}
