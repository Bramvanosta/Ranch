//! Jobs of a queue: the paginated list per state (plain or searched /
//! sorted), one job by id, its log lines, and the jobs that ran last.

use std::collections::HashMap;

use tokio_util::sync::CancellationToken;

use super::queues::list_queues_with_refs;
use super::{read_each, redis_value_hash_to_bytes};
use crate::bullmq_schema::job_query::{self, JobQuery, JobRow};
use crate::bullmq_schema::states;
use crate::bullmq_schema::v5::V5Decoder;
use crate::bullmq_schema::v6::{V6Decoder, MARKER_TOKEN};
use crate::bullmq_schema::{
    BullmqSchema, Job, JobDecoder, JobState, KeyVariant, Keys, QueueRef, RecentJobs,
    StateRedisShape,
};
use crate::connection_actor::actor::{lock, ActorState};
use crate::connection_actor::handle::QueryTicket;
use crate::connection_actor::link::{
    open_redis_for_actor, redis_to_ranch_err, resolve_keys_for_queue, RedisConn,
};
use ranch_domain::errors::RanchError;
use ranch_domain::jobs::{
    JobLogs, JobLogsQuery, JobPage, JobPageQuery, JobTarget, RecentJobsQuery,
};

/// The largest page of jobs or log lines one read returns (the UI offers up
/// to 250 jobs a page, the MCP tools cap lower).
const MAX_PAGE_SIZE: u64 = 1_000;

/// Far beyond any real queue, and far from `i64::MAX`: Redis ranges are signed.
const MAX_OFFSET: u64 = 1 << 40;

/// Pick the right Redis key for a given state via the cached `KeyVariant`.
fn key_for_state(keys: KeyVariant, qref: &QueueRef, state: JobState) -> String {
    match state {
        JobState::Waiting => keys.wait(qref),
        JobState::Active => keys.active(qref),
        JobState::Completed => keys.completed(qref),
        JobState::Failed => keys.failed(qref),
        JobState::Delayed => keys.delayed(qref),
        states::PRIORITIZED => keys.prioritized(qref),
        states::WAITING_CHILDREN => keys.waiting_children(qref),
        states::STALLED => keys.stalled(qref),
        JobState::Paused => keys.paused(qref),
        // No other state reaches here (handle_list_jobs short-circuits
        // earlier), but we return a never-existing key so an accidental
        // call returns empty rather than panicking.
        JobState::Other(_) => format!("{}:{}::ranch-unknown-state", qref.prefix, qref.name),
    }
}

/// Discover + decode a paginated slice of jobs for a (queue,
/// state) pair. See module-level docs for the contract.
pub(crate) async fn handle_list_jobs(
    state: &mut ActorState,
    args: &JobPageQuery,
    ticket: Option<&QueryTicket>,
    token: &CancellationToken,
) -> Result<JobPage, RanchError> {
    let domain_queue = QueueRef::parse(args.queue_id.as_str())?;
    // Typed three more letters while this waited in the
    // mailbox: don't even open the scan.
    if ticket.is_some_and(QueryTicket::is_superseded) {
        return Err(RanchError::Superseded);
    }
    // The SSH tunnel, when there is one, is opened inside `open_redis_for_actor`.
    let mut conn = open_redis_for_actor(state, token).await?;

    // Resolve the schema → KeyVariant (cached from prior list_queues, or
    // detect on the fly via the queue's meta).
    let keys = resolve_keys_for_queue(state, &mut conn, &domain_queue, token).await?;
    let job_state = args.state;
    let key = key_for_state(keys, &domain_queue, job_state);

    // Detect whether we're reading from a list (LRANGE) or sorted set
    // (ZRANGE/ZREVRANGE). Unsupported states return empty quickly.
    let shape = StateRedisShape::of(job_state);
    if shape == StateRedisShape::Unsupported {
        return Ok(JobPage {
            jobs: Vec::new(),
            total: 0,
            state: job_state,
            has_more: false,
            scanned: None,
            scan_capped: false,
        });
    }

    // A search, a reason filter or a custom sort has to look at
    // the jobs themselves: bounded scan instead of a range read.
    let query = JobQuery {
        search: args.search.clone(),
        reason: args.reason.clone(),
        sort: args.sort,
    };
    if query.is_active() {
        return list_jobs_by_query(&mut conn, keys, &key, shape, args, &query, ticket, token).await;
    }

    // Page slice. Bounded whoever asks (an agent's `limit: 10000000` would
    // range-read a whole production set); saturating, so no overflow wraps
    // into Redis's "-1 = to the end".
    let limit = args.limit.clamp(1, MAX_PAGE_SIZE);
    let offset = args.offset.min(MAX_OFFSET);
    let stop = offset.saturating_add(limit - 1); // inclusive in Redis range syntax

    // The total and the page in one round trip. ZREVRANGE for
    // newest-first (most-recent timestamp first), ZRANGE otherwise.
    let (count_cmd, range_cmd) = match shape {
        StateRedisShape::List => ("LLEN", "LRANGE"),
        _ if args.newest_first => ("ZCARD", "ZREVRANGE"),
        _ => ("ZCARD", "ZRANGE"),
    };
    let mut pipe = redis::pipe();
    pipe.cmd(count_cmd)
        .arg(&key)
        .cmd(range_cmd)
        .arg(&key)
        .arg(offset as i64)
        .arg(stop as i64);
    let (total, raw_ids): (u64, Vec<String>) = tokio::select! {
        r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };

    // For v6 list states with marker, subtract 1.
    let v6_marker_state = matches!(*lock(&state.cached_schema), Some(BullmqSchema::V6))
        && shape == StateRedisShape::List
        && matches!(
            job_state,
            JobState::Waiting | states::PRIORITIZED | JobState::Delayed
        );
    let total_adjusted = if v6_marker_state && total > 0 {
        total - 1
    } else {
        total
    };

    // Filter out the BullMQ v6 marker token. The marker only appears in
    // v6 list states; passing the bytes-equal check is safe everywhere.
    let job_ids: Vec<String> = raw_ids
        .into_iter()
        .filter(|id| id.as_bytes() != MARKER_TOKEN)
        .collect();

    let jobs = fetch_jobs_by_ids(&mut conn, keys, args, &job_ids, token).await?;

    let returned = jobs.len() as u64;
    let has_more = offset + returned < total_adjusted;

    Ok(JobPage {
        jobs,
        total: total_adjusted,
        state: job_state,
        has_more,
        scanned: None,
        scan_capped: false,
    })
}

/// `HGETALL` + decode the given ids, in order. Per-job resilience: a job
/// deleted mid-flight or failing to decode is skipped with a warning.
async fn fetch_jobs_by_ids(
    conn: &mut RedisConn,
    keys: KeyVariant,
    args: &JobPageQuery,
    job_ids: &[String],
    token: &CancellationToken,
) -> Result<Vec<Job>, RanchError> {
    let queue = QueueRef::parse(args.queue_id.as_str())?;
    // Build the per-job HGETALL pipeline (non-atomic: independent reads,
    // we just want pipelining round-trip reduction).
    let job_hash_keys: Vec<String> = job_ids
        .iter()
        .map(|id| {
            let jref = queue.job(id.clone());
            keys.job_hash(&jref)
        })
        .collect();

    // A running job's lock: how long until it expires (the
    // worker renews it while the job runs). Read with the
    // hashes, in the same round trip.
    let with_locks = args.state == JobState::Active;
    let mut pipe = redis::pipe();
    for hk in &job_hash_keys {
        pipe.cmd("HGETALL").arg(hk);
    }
    if with_locks {
        for hk in &job_hash_keys {
            pipe.cmd("PTTL").arg(format!("{hk}:lock"));
        }
    }
    let mut replies: Vec<redis::Value> = if job_hash_keys.is_empty() {
        Vec::new()
    } else {
        tokio::select! {
            r = pipe.query_async(&mut *conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => return Err(RanchError::Cancelled),
        }
    };
    let lock_ttls: Vec<Option<i64>> = if with_locks {
        replies
            .split_off(job_hash_keys.len().min(replies.len()))
            .into_iter()
            .map(|v| redis::from_redis_value::<i64>(v).ok().filter(|t| *t >= 0))
            .collect()
    } else {
        Vec::new()
    };
    let raw_hashes: Vec<HashMap<String, redis::Value>> = replies
        .into_iter()
        .map(|v| redis::from_redis_value(v).unwrap_or_default())
        .collect();

    // Decode each hash via the cached schema's JobDecoder. Per-job
    // resilience: empty hash (job deleted mid-flight) → skip + warn;
    // decode error → skip + warn.
    let decoder: Box<dyn JobDecoder + Send + Sync> = match keys {
        KeyVariant::V5(_) => Box::new(V5Decoder),
        KeyVariant::V6(_) => Box::new(V6Decoder),
    };
    let mut jobs: Vec<Job> = Vec::with_capacity(job_ids.len());
    for (i, (id, raw)) in job_ids.iter().zip(raw_hashes).enumerate() {
        if raw.is_empty() {
            tracing::warn!(
                queue = %args.queue_id,
                job_id = %id,
                "job hash empty (deleted mid-flight); skipping"
            );
            continue;
        }
        let fields = redis_value_hash_to_bytes(raw);
        let jref = queue.job(id.clone());
        match decoder.decode_job(jref, fields) {
            Ok(mut job) => {
                // Override the heuristic state with the queried state:
                // the membership-query is the authoritative source.
                job.state = args.state;
                job.lock_ttl_ms = lock_ttls.get(i).copied().flatten();
                jobs.push(job);
            }
            Err(e) => {
                tracing::warn!(
                    queue = %args.queue_id,
                    job_id = %id,
                    error = %e,
                    "job decode failed; skipping"
                );
            }
        }
    }

    Ok(jobs)
}

/// Search / sort across a whole state. The deciding
/// logic is pure (`bullmq_schema::job_query`); this fetches:
///
///   1. the newest `SCAN_CAP` ids of the state, in its natural order;
///   2. `HMGET name timestamp processedOn finishedOn failedReason` for each,
///      in pipelines of `SCAN_CHUNK`, cancellable between chunks;
///   3. the full hash of the jobs on the requested page only.
///
/// A newer query on the same connection stops this one between two chunks
/// (`RanchError::Superseded`).
#[allow(clippy::too_many_arguments)]
async fn list_jobs_by_query(
    conn: &mut RedisConn,
    keys: KeyVariant,
    key: &str,
    shape: StateRedisShape,
    args: &JobPageQuery,
    query: &JobQuery,
    ticket: Option<&QueryTicket>,
    token: &CancellationToken,
) -> Result<JobPage, RanchError> {
    let cancelled = || RanchError::Cancelled;
    let queue = QueueRef::parse(args.queue_id.as_str())?;
    let state_total = fetch_total(conn, key, shape, token).await?;

    // +1: a v6 marker token may take one slot of a list state.
    let stop = job_query::SCAN_CAP as i64;
    let cmd_name = match shape {
        StateRedisShape::List => "LRANGE",
        StateRedisShape::SortedSet if args.newest_first => "ZREVRANGE",
        StateRedisShape::SortedSet => "ZRANGE",
        StateRedisShape::Unsupported => unreachable!("short-circuited by handle_list_jobs"),
    };
    let mut cmd = redis::cmd(cmd_name);
    cmd.arg(key).arg(0).arg(stop);
    let raw_ids: Vec<String> = tokio::select! {
        r = cmd.query_async::<Vec<String>>(&mut *conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(cancelled()),
    };
    let had_marker = raw_ids.iter().any(|id| id.as_bytes() == MARKER_TOKEN);
    let ids: Vec<String> = raw_ids
        .into_iter()
        .filter(|id| id.as_bytes() != MARKER_TOKEN)
        .take(job_query::SCAN_CAP)
        .collect();

    let mut rows: Vec<JobRow> = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(job_query::SCAN_CHUNK) {
        if token.is_cancelled() {
            return Err(cancelled());
        }
        if ticket.is_some_and(QueryTicket::is_superseded) {
            return Err(RanchError::Superseded);
        }
        let mut pipe = redis::pipe();
        for id in chunk {
            let jref = queue.job(id.clone());
            pipe.cmd("HMGET")
                .arg(keys.job_hash(&jref))
                .arg("name")
                .arg("timestamp")
                .arg("processedOn")
                .arg("finishedOn")
                .arg("failedReason");
        }
        let fetched: Vec<Vec<Option<String>>> = tokio::select! {
            r = pipe.query_async(&mut *conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => return Err(cancelled()),
        };
        for (id, fields) in chunk.iter().zip(fetched) {
            let mut fields = fields.into_iter();
            let mut next = || fields.next().flatten();
            let name = next();
            let timestamp = next().and_then(|v| v.parse::<i64>().ok());
            let processed_on = next().and_then(|v| v.parse::<i64>().ok());
            let finished_on = next().and_then(|v| v.parse::<i64>().ok());
            let failed_reason = next().filter(|r| !r.is_empty());
            // No name and no timestamp = the hash is gone (removed mid-scan).
            if name.is_none() && timestamp.is_none() {
                continue;
            }
            rows.push(JobRow {
                id: id.clone(),
                name: name.unwrap_or_default(),
                timestamp,
                processed_on,
                finished_on,
                failed_reason,
            });
        }
    }

    let scanned = rows.len() as u64;
    let (page_ids, matching) = query.run(rows, args.offset, args.limit);
    let jobs = fetch_jobs_by_ids(conn, keys, args, &page_ids, token).await?;
    let has_more = args.offset + (page_ids.len() as u64) < matching;
    let state_jobs = state_total.saturating_sub(u64::from(had_marker));

    Ok(JobPage {
        jobs,
        total: matching,
        state: args.state,
        has_more,
        scanned: Some(scanned),
        scan_capped: state_jobs > job_query::SCAN_CAP as u64,
    })
}

/// The jobs that ran last, on one queue or across the connection:
///
///   1. the queues (one, or every queue that has something running or finished);
///   2. per queue, one pipeline: the newest ids of `completed` and `failed`
///      with their scores, and the head of `active`;
///   3. `HGETALL` for the `limit` latest finished jobs and the running ones,
///      in one pipeline.
///
/// A queue that cannot be read is reported in `unreadable_queues`, never
/// fatal.
pub(crate) async fn handle_list_recent_jobs(
    state: &mut ActorState,
    args: &RecentJobsQuery,
    token: &CancellationToken,
) -> Result<RecentJobs, RanchError> {
    use crate::bullmq_schema::recent::{
        clamp_limit, newest_finished, order_recent, FinishedCandidate, MAX_ACTIVE,
    };
    let cancelled = || RanchError::Cancelled;
    let limit = clamp_limit(args.limit);

    // 1. Which queues.
    let queues: Vec<QueueRef> = match &args.queue_id {
        Some(id) => vec![QueueRef::parse(id.as_str())?],
        None => list_queues_with_refs(state, token)
            .await?
            .into_iter()
            .filter(|(_, q)| {
                [JobState::Active, JobState::Completed, JobState::Failed]
                    .into_iter()
                    .any(|s| q.counts.get(s) > 0)
            })
            .map(|(qref, _)| qref)
            .collect(),
    };
    if queues.is_empty() {
        return Ok(RecentJobs::default());
    }

    let mut conn = open_redis_for_actor(state, token).await?;

    // 2. Newest finished ids + running ids, per queue.
    let mut unreadable_queues = Vec::new();
    let mut finished: Vec<FinishedCandidate> = Vec::new();
    let mut running: Vec<(QueueRef, String)> = Vec::new();
    let mut keys_of: HashMap<QueueRef, KeyVariant> = HashMap::new();
    // Every queue's reads at once.
    let mut readable: Vec<(QueueRef, KeyVariant)> = Vec::new();
    for q in &queues {
        match resolve_keys_for_queue(state, &mut conn, q, token).await {
            Ok(keys) => readable.push((q.clone(), keys)),
            Err(_) => unreadable_queues.push(q.name.clone()),
        }
    }
    type Scored = Vec<(String, f64)>;
    let replies = read_each(
        &conn,
        readable.clone(),
        token,
        |mut conn, (q, keys): (QueueRef, KeyVariant)| async move {
            let mut pipe = redis::pipe();
            for key in [keys.completed(&q), keys.failed(&q)] {
                pipe.cmd("ZREVRANGE")
                    .arg(key)
                    .arg(0_i64)
                    .arg(limit as i64 - 1)
                    .arg("WITHSCORES");
            }
            pipe.cmd("LRANGE")
                .arg(keys.active(&q))
                .arg(0_i64)
                .arg(limit as i64 - 1);
            pipe.query_async::<(Scored, Scored, Vec<String>)>(&mut conn)
                .await
        },
    )
    .await?;
    for ((q, keys), read) in readable.iter().zip(replies) {
        let (q, keys) = (q, *keys);
        let Some(Ok((completed, failed, active))) = read else {
            unreadable_queues.push(q.name.clone());
            continue;
        };
        keys_of.insert(q.clone(), keys);
        for (ids, job_state) in [(completed, JobState::Completed), (failed, JobState::Failed)] {
            finished.extend(ids.into_iter().map(|(id, score)| FinishedCandidate {
                queue: q.clone(),
                job_id: id,
                state: job_state,
                finished_at: score as i64,
            }));
        }
        running.extend(active.into_iter().map(|id| (q.clone(), id)));
    }
    running.truncate(MAX_ACTIVE);

    // 3. Full hashes of what will be shown.
    let mut wanted: Vec<(QueueRef, String, JobState)> = running
        .into_iter()
        .map(|(q, id)| (q, id, JobState::Active))
        .collect();
    wanted.extend(
        newest_finished(finished, limit)
            .into_iter()
            .map(|c| (c.queue, c.job_id, c.state)),
    );
    let mut jobs: Vec<Job> = Vec::with_capacity(wanted.len());
    if !wanted.is_empty() {
        let mut pipe = redis::pipe();
        for (q, id, _) in &wanted {
            pipe.cmd("HGETALL")
                .arg(keys_of[q].job_hash(&q.job(id.clone())));
        }
        let hashes: Vec<HashMap<String, redis::Value>> = tokio::select! {
            r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => return Err(cancelled()),
        };
        for ((q, id, job_state), hash) in wanted.into_iter().zip(hashes) {
            if hash.is_empty() {
                continue; // removed (removeOnComplete…) between the two reads
            }
            let decoder: Box<dyn JobDecoder + Send + Sync> = match keys_of[&q] {
                KeyVariant::V5(_) => Box::new(V5Decoder),
                KeyVariant::V6(_) => Box::new(V6Decoder),
            };
            let jref = q.job(id.clone());
            match decoder.decode_job(jref, redis_value_hash_to_bytes(hash)) {
                Ok(mut job) => {
                    job.state = job_state; // set membership is the authority
                    jobs.push(job);
                }
                Err(e) => tracing::warn!(
                    queue = %q.name, job_id = %id, error = %e,
                    "recent-job decode failed; skipping"
                ),
            }
        }
    }

    Ok(RecentJobs {
        jobs: order_recent(jobs, limit),
        unreadable_queues,
    })
}

/// Fetch total count for a state via LLEN or ZCARD.
async fn fetch_total(
    conn: &mut RedisConn,
    key: &str,
    shape: StateRedisShape,
    token: &CancellationToken,
) -> Result<u64, RanchError> {
    let cmd_name = match shape {
        StateRedisShape::List => "LLEN",
        StateRedisShape::SortedSet => "ZCARD",
        StateRedisShape::Unsupported => return Ok(0),
    };
    let mut cmd = redis::cmd(cmd_name);
    cmd.arg(key);
    tokio::select! {
        r = cmd.query_async::<u64>(conn) => r.map_err(redis_to_ranch_err),
        _ = token.cancelled() => Err(RanchError::Cancelled),
    }
}

/// Fetch + decode a single job for the drill-down route.
/// One HGETALL on `bull:{queue}:{id}` + a `JobDecoder::decode_job` call.
/// Returns `RanchError::JobNotFound` on an empty hash so the UI can render
/// an explicit 404-style empty state without retrying.
pub(crate) async fn handle_get_job(
    state: &mut ActorState,
    args: &JobTarget,
    token: &CancellationToken,
) -> Result<Job, RanchError> {
    let domain_queue = QueueRef::parse(args.queue_id.as_str())?;
    // The SSH tunnel, when there is one, is opened inside `open_redis_for_actor`.
    let mut conn = open_redis_for_actor(state, token).await?;

    let keys = resolve_keys_for_queue(state, &mut conn, &domain_queue, token).await?;
    let jref = domain_queue.job(args.job_id.clone());
    let key = keys.job_hash(&jref);

    // The hash and its lock (a job that started and
    // has not finished may hold one) in one round trip.
    let mut pipe = redis::pipe();
    pipe.cmd("HGETALL")
        .arg(&key)
        .cmd("PTTL")
        .arg(format!("{key}:lock"));
    let (raw, lock_ttl): (HashMap<String, redis::Value>, i64) = tokio::select! {
        r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::Cancelled);
        }
    };

    if raw.is_empty() {
        return Err(RanchError::JobNotFound {
            queue: domain_queue.name.clone(),
            id: args.job_id.clone(),
        });
    }

    let fields = redis_value_hash_to_bytes(raw);
    let decoder: Box<dyn JobDecoder + Send + Sync> = match keys {
        KeyVariant::V5(_) => Box::new(V5Decoder),
        KeyVariant::V6(_) => Box::new(V6Decoder),
    };
    let mut job = decoder
        .decode_job(jref, fields)
        .map_err(|e| RanchError::InvalidInput {
            reason: format!(
                "Failed to decode job {}/{}: {e}",
                domain_queue.name, args.job_id
            ),
        })?;
    if job.processed_on.is_some() && job.finished_on.is_none() {
        job.lock_ttl_ms = (lock_ttl >= 0).then_some(lock_ttl);
    }
    Ok(job)
}

/// Read BullMQ job logs from the `bull:{q}:{id}:logs` list.
/// `Keys::logs(jref)` already gives the right key shape
/// for both v5 and v6 (identical).
pub(crate) async fn handle_get_job_logs(
    state: &mut ActorState,
    args: &JobLogsQuery,
    token: &CancellationToken,
) -> Result<JobLogs, RanchError> {
    let domain_queue = QueueRef::parse(args.queue_id.as_str())?;
    // The SSH tunnel, when there is one, is opened inside `open_redis_for_actor`.
    let mut conn = open_redis_for_actor(state, token).await?;

    let keys = resolve_keys_for_queue(state, &mut conn, &domain_queue, token).await?;
    let jref = domain_queue.job(args.job_id.clone());
    let key = keys.logs(&jref);

    // LLEN: returns 0 cleanly when the key doesn't exist.
    let mut llen_cmd = redis::cmd("LLEN");
    llen_cmd.arg(&key);
    let total: u64 = tokio::select! {
        r = llen_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::Cancelled);
        }
    };

    if total == 0 {
        return Ok(JobLogs {
            lines: Vec::new(),
            total: 0,
            has_more: false,
        });
    }

    let limit = args.limit.clamp(1, MAX_PAGE_SIZE);
    let offset = args.offset.min(MAX_OFFSET);
    let stop = offset.saturating_add(limit - 1);

    let mut lrange_cmd = redis::cmd("LRANGE");
    lrange_cmd.arg(&key).arg(offset as i64).arg(stop as i64);
    let raw_lines: Vec<Vec<u8>> = tokio::select! {
        r = lrange_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::Cancelled);
        }
    };

    // Lossy UTF-8: BullMQ stores log lines as UTF-8 strings; the rare
    // embedded-binary case shouldn't fail the whole call.
    let lines: Vec<String> = raw_lines
        .into_iter()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .collect();
    let has_more = offset + (lines.len() as u64) < total;

    Ok(JobLogs {
        lines,
        total,
        has_more,
    })
}
