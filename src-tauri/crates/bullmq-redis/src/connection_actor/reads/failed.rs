//! Failed jobs across the connection's queues: the bounded scan of the
//! newest failures, the paginated failed list, and the failures grouped
//! by cause.

use std::collections::HashMap;

use tokio_util::sync::CancellationToken;

use super::queues::list_queues_with_refs;
use super::{read_each, read_panicked, redis_value_hash_to_bytes};
use crate::bullmq_schema::v5::V5Decoder;
use crate::bullmq_schema::v6::V6Decoder;
use crate::bullmq_schema::{Job, JobDecoder, JobRef, JobState, KeyVariant, Keys, QueueRef};
use crate::connection_actor::actor::ActorState;
use crate::connection_actor::link::{
    open_redis_for_actor, redis_to_ranch_err, resolve_keys_for_queue, RedisConn,
};
use ranch_domain::errors::RanchError;
use ranch_domain::failures::{FailedJobsPage, FailedJobsQuery, FailureCauses};

/// What the failed views read: the queues' exact
/// failed counts and the newest failed jobs, bounded (`domain::failures`).
///
///   1. the queues (`handle_list_queues`, which also caches the schema);
///   2. per queue with failures: the newest `PER_QUEUE_CAP` ids of its
///      `failed` set, with their scores, in one pipeline;
///   3. `HMGET name failedReason` for every candidate, in one pipeline (the
///      summaries and the text filters need them).
///
/// A queue whose failed set cannot be read is reported in
/// `unreadable_queues`, never fatal.
struct FailedScan {
    counts: Vec<ranch_domain::failures::FailedQueueCount>,
    total_failed: u32,
    raw: Vec<(QueueRef, KeyVariant, String, i64)>,
    candidates: Vec<ranch_domain::failures::FailedCandidate>,
    scan_capped: bool,
    unreadable_queues: Vec<String>,
}

async fn scan_failed(
    state: &mut ActorState,
    only_queue: Option<&str>,
    token: &CancellationToken,
) -> Result<Option<(FailedScan, RedisConn)>, RanchError> {
    use ranch_domain::failures::{
        merge_candidates, FailedCandidate, FailedQueueCount, PER_QUEUE_CAP,
    };
    let cancelled = || RanchError::Cancelled;

    // 1. Queues + exact failed counts.
    let queues = list_queues_with_refs(state, token).await?;
    let mut failing: Vec<(QueueRef, FailedQueueCount)> = queues
        .into_iter()
        .filter(|(_, q)| q.counts.get(JobState::Failed) > 0)
        .filter(|(_, q)| only_queue.is_none_or(|name| q.name == name))
        .map(|(qref, q)| {
            let count = FailedQueueCount {
                queue_id: q.id,
                queue_name: q.name,
                failed: q.counts.get(JobState::Failed).min(u32::MAX as u64) as u32,
            };
            (qref, count)
        })
        .collect();
    failing.sort_by(|(_, a), (_, b)| {
        b.failed
            .cmp(&a.failed)
            .then(a.queue_name.cmp(&b.queue_name))
    });
    let total_failed: u32 = failing.iter().map(|(_, c)| c.failed).sum();
    if failing.is_empty() {
        return Ok(None);
    }

    let mut conn = open_redis_for_actor(state, token).await?;

    // 2. Newest failed ids per queue, every queue at once.
    let mut unreadable_queues = Vec::new();
    let mut raw: Vec<(QueueRef, KeyVariant, String, i64)> = Vec::new();
    let mut scan_capped = false;
    let mut readable: Vec<(&QueueRef, &FailedQueueCount, KeyVariant)> = Vec::new();
    for (qref, count) in &failing {
        match resolve_keys_for_queue(state, &mut conn, qref, token).await {
            Ok(keys) => readable.push((qref, count, keys)),
            Err(_) => unreadable_queues.push(qref.name.clone()),
        }
    }
    let reads = readable
        .iter()
        .map(|(qref, _, keys)| keys.failed(qref))
        .collect();
    let replies = read_each(&conn, reads, token, |mut conn, key: String| async move {
        let mut cmd = redis::cmd("ZREVRANGE");
        cmd.arg(key)
            .arg(0_i64)
            .arg(PER_QUEUE_CAP as i64 - 1)
            .arg("WITHSCORES");
        cmd.query_async::<Vec<(String, f64)>>(&mut conn).await
    })
    .await?;
    for ((q, count, keys), ids) in readable.into_iter().zip(replies) {
        let keys = &keys;
        match ids.unwrap_or_else(|| Err(read_panicked())) {
            Ok(ids) => {
                if count.failed as usize > ids.len() {
                    scan_capped = true;
                }
                for (id, score) in ids {
                    raw.push((q.clone(), *keys, id, score as i64));
                }
            }
            Err(_) => unreadable_queues.push(q.name.clone()),
        }
    }

    // 3. name + failedReason of every candidate.
    let mut candidates: Vec<FailedCandidate> = Vec::with_capacity(raw.len());
    if !raw.is_empty() {
        let mut pipe = redis::pipe();
        for (q, keys, id, _) in &raw {
            pipe.cmd("HMGET")
                .arg(keys.job_hash(&q.job(id.clone())))
                .arg("name")
                .arg("failedReason");
        }
        let rows: Vec<Vec<Option<String>>> = tokio::select! {
            r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => return Err(cancelled()),
        };
        for ((q, _, id, score), fields) in raw.iter().zip(rows) {
            // No name = the hash is gone (removed mid-flight): not a failed job anymore.
            let Some(name) = fields.first().cloned().flatten() else {
                continue;
            };
            candidates.push(FailedCandidate {
                queue_id: q.id(),
                queue_name: q.name.clone(),
                job_id: id.clone(),
                failed_at: *score,
                name,
                reason: fields.get(1).cloned().flatten().filter(|s| !s.is_empty()),
            });
        }
    }
    let before_cap = candidates.len();
    let candidates = merge_candidates(candidates);
    if candidates.len() < before_cap {
        scan_capped = true;
    }
    Ok(Some((
        FailedScan {
            counts: failing.into_iter().map(|(_, count)| count).collect(),
            total_failed,
            raw,
            candidates,
            scan_capped,
            unreadable_queues,
        },
        conn,
    )))
}

/// Everything that failed on this connection, newest first: the scan ([`scan_failed`]), then `HGETALL` for the jobs of the
/// requested page only.
pub(crate) async fn handle_list_failed_jobs(
    state: &mut ActorState,
    args: &FailedJobsQuery,
    token: &CancellationToken,
) -> Result<FailedJobsPage, RanchError> {
    use ranch_domain::failures::{page_of, summarise_types, FailedCandidate};
    let cancelled = || RanchError::Cancelled;
    let Some((scan, mut conn)) = scan_failed(state, None, token).await? else {
        return Ok(FailedJobsPage::default());
    };
    let FailedScan {
        counts,
        total_failed,
        raw,
        candidates,
        scan_capped,
        unreadable_queues,
    } = scan;

    let filter = &args.filter;
    let types = summarise_types(
        &candidates,
        filter
            .queue
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    );
    let (page, total) = page_of(&candidates, filter, args.offset, args.limit);

    // 4. Full jobs for the page only.
    let mut jobs: Vec<Job> = Vec::with_capacity(page.len());
    if !page.is_empty() {
        let key_of = |c: &FailedCandidate| {
            raw.iter()
                .find(|(q, _, id, _)| q.id() == c.queue_id && id == &c.job_id)
                .map(|(_, keys, _, _)| *keys)
        };
        let mut pipe = redis::pipe();
        let jref = |c: &FailedCandidate| JobRef {
            queue_id: c.queue_id.clone(),
            queue_name: c.queue_name.clone(),
            id: c.job_id.clone(),
        };
        let mut wanted: Vec<(&FailedCandidate, KeyVariant)> = Vec::new();
        for c in &page {
            if let Some(keys) = key_of(c) {
                pipe.cmd("HGETALL").arg(keys.job_hash(&jref(c)));
                wanted.push((c, keys));
            }
        }
        let hashes: Vec<HashMap<String, redis::Value>> = tokio::select! {
            r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => return Err(cancelled()),
        };
        for ((c, keys), hash) in wanted.into_iter().zip(hashes) {
            if hash.is_empty() {
                continue; // removed between step 3 and step 4
            }
            let decoder: Box<dyn JobDecoder + Send + Sync> = match keys {
                KeyVariant::V5(_) => Box::new(V5Decoder),
                KeyVariant::V6(_) => Box::new(V6Decoder),
            };
            match decoder.decode_job(jref(c), redis_value_hash_to_bytes(hash)) {
                Ok(mut job) => {
                    job.state = JobState::Failed;
                    jobs.push(job);
                }
                Err(e) => tracing::warn!(
                    queue = %c.queue_name, job_id = %c.job_id, error = %e,
                    "failed-job decode failed; skipping"
                ),
            }
        }
    }

    Ok(FailedJobsPage {
        jobs,
        total,
        total_failed,
        queues: counts,
        types,
        scanned: candidates.len() as u32,
        scan_capped,
        unreadable_queues,
    })
}

/// The failed jobs of a queue (or of the connection) grouped
/// by cause: the same bounded scan as the failed view.
pub(crate) async fn handle_list_failure_causes(
    state: &mut ActorState,
    queue: Option<&str>,
    token: &CancellationToken,
) -> Result<FailureCauses, RanchError> {
    use ranch_domain::failures::group_by_cause;
    let queue = queue.map(str::trim).filter(|s| !s.is_empty());
    let Some((scan, _conn)) = scan_failed(state, queue, token).await? else {
        return Ok(FailureCauses::default());
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Ok(FailureCauses {
        causes: group_by_cause(&scan.candidates, queue, now_ms),
        scanned: scan.candidates.len() as u32,
        scan_capped: scan.scan_capped,
        total_failed: scan.total_failed,
        unreadable_queues: scan.unreadable_queues,
    })
}
