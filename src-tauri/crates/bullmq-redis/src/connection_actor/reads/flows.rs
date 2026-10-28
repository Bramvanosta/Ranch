//! Flows: the parent / child tree a job belongs to and the flow roots of
//! a queue, read through bounded probes of each job's Redis containers.

use tokio_util::sync::CancellationToken;

use crate::bullmq_schema::flow::{self, FlowNode, FlowRoot, FlowRoots, FlowTree};
use crate::bullmq_schema::states;
use crate::bullmq_schema::QueueRef;
use crate::bullmq_schema::{JobRef, JobState, KeyVariant, Keys};
use crate::connection_actor::actor::ActorState;
use crate::connection_actor::link::{
    open_redis_for_actor, redis_to_ranch_err, resolve_keys_for_queue, RedisConn,
};
use ranch_domain::errors::RanchError;
use ranch_domain::jobs::JobTarget;
use ranch_domain::queues::QueueId;

/// What one job of a flow looks like in Redis.
struct FlowProbe {
    exists: bool,
    name: String,
    timestamp: Option<i64>,
    duration_ms: Option<i64>,
    failed_reason: Option<String>,
    parent_key: Option<String>,
    /// Raw `opts` and `returnvalue`.
    opts: Option<String>,
    return_value: Option<String>,
    state: JobState,
    /// Keys of its direct children, whatever became of them.
    children: Vec<String>,
    /// Children the parent recorded as processed / as failed; it remembers
    /// them even once `removeOnComplete` has deleted their hash.
    completed: u64,
    failed: u64,
}

/// Commands per job in [`probe_flow_jobs`].
const FLOW_PROBE_WIDTH: usize = 14;

/// Read `refs` in ONE pipeline: fields, state (container
/// membership) and the four children keys. A flow of 200 jobs is a handful
/// of round trips (one per tree level), not 200.
async fn probe_flow_jobs(
    conn: &mut RedisConn,
    keys: KeyVariant,
    refs: &[JobRef],
) -> Result<Vec<FlowProbe>, redis::RedisError> {
    if refs.is_empty() {
        return Ok(Vec::new());
    }
    let mut pipe = redis::pipe();
    for jref in refs {
        let q = &QueueRef::from_id(&jref.queue_id).map_err(|_| {
            redis::RedisError::from((redis::ErrorKind::Client, "not a BullMQ queue id"))
        })?;
        let job_key = keys.job_hash(jref);
        pipe.cmd("HMGET")
            .arg(&job_key)
            .arg("name")
            .arg("timestamp")
            .arg("processedOn")
            .arg("finishedOn")
            .arg("failedReason")
            .arg("parentKey")
            .arg("opts")
            .arg("returnvalue");
        pipe.cmd("LPOS").arg(keys.wait(q)).arg(&jref.id);
        pipe.cmd("LPOS").arg(keys.active(q)).arg(&jref.id);
        pipe.cmd("LPOS").arg(keys.paused(q)).arg(&jref.id);
        pipe.cmd("ZSCORE").arg(keys.completed(q)).arg(&jref.id);
        pipe.cmd("ZSCORE").arg(keys.failed(q)).arg(&jref.id);
        pipe.cmd("ZSCORE").arg(keys.delayed(q)).arg(&jref.id);
        pipe.cmd("ZSCORE").arg(keys.prioritized(q)).arg(&jref.id);
        pipe.cmd("ZSCORE")
            .arg(keys.waiting_children(q))
            .arg(&jref.id);
        pipe.cmd("EXISTS").arg(&job_key);
        pipe.cmd("SMEMBERS").arg(format!("{job_key}:dependencies"));
        pipe.cmd("HKEYS").arg(format!("{job_key}:processed"));
        pipe.cmd("HKEYS").arg(format!("{job_key}:failed"));
        pipe.cmd("ZRANGE")
            .arg(format!("{job_key}:unsuccessful"))
            .arg(0)
            .arg(-1);
    }
    let raw: Vec<redis::Value> = pipe.query_async(conn).await?;
    let mut probes = Vec::with_capacity(refs.len());
    for chunk in raw.chunks(FLOW_PROBE_WIDTH) {
        if chunk.len() < FLOW_PROBE_WIDTH {
            break;
        }
        let get = |i: usize| chunk[i].clone();
        let fields: Vec<Option<String>> = redis::from_redis_value(get(0))?;
        let field = |i: usize| fields.get(i).cloned().flatten().filter(|v| !v.is_empty());
        let num = |i: usize| field(i).and_then(|v| v.parse::<i64>().ok());
        let present = |i: usize| !matches!(chunk[i], redis::Value::Nil);
        let state = if present(1) {
            JobState::Waiting
        } else if present(2) {
            JobState::Active
        } else if present(3) {
            JobState::Paused
        } else if present(5) {
            JobState::Failed
        } else if present(6) {
            JobState::Delayed
        } else if present(4) {
            JobState::Completed
        } else if present(7) {
            states::PRIORITIZED
        } else if present(8) {
            states::WAITING_CHILDREN
        } else {
            states::UNKNOWN
        };
        let exists: bool = redis::from_redis_value(get(9))?;
        let pending: Vec<String> = redis::from_redis_value(get(10))?;
        let processed: Vec<String> = redis::from_redis_value(get(11))?;
        let ignored: Vec<String> = redis::from_redis_value(get(12))?;
        let unsuccessful: Vec<String> = redis::from_redis_value(get(13))?;
        let duration_ms = match (num(2), num(3)) {
            (Some(p), Some(f)) if f >= p => Some(f - p),
            _ => None,
        };
        let counts = (
            processed.len() as u64,
            (ignored.len() + unsuccessful.len()) as u64,
        );
        let mut children = pending;
        for key in processed.into_iter().chain(ignored).chain(unsuccessful) {
            if !children.contains(&key) {
                children.push(key);
            }
        }
        children.sort();
        probes.push(FlowProbe {
            exists,
            name: field(0).unwrap_or_default(),
            timestamp: num(1),
            duration_ms,
            failed_reason: field(4),
            parent_key: field(5),
            opts: field(6),
            return_value: field(7),
            state,
            children,
            completed: counts.0,
            failed: counts.1,
        });
    }
    Ok(probes)
}

/// The flow a job belongs to: up to the root through
/// `parentKey`, then down level by level through each parent's children
/// keys. Bounded (`flow::MAX_NODES`, `flow::MAX_ANCESTORS`); a job that is
/// in no flow comes back as a tree of one.
pub(crate) async fn handle_get_flow(
    state: &mut ActorState,
    args: &JobTarget,
    token: &CancellationToken,
) -> Result<FlowTree, RanchError> {
    let domain_queue = QueueRef::parse(args.queue_id.as_str())?;
    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &domain_queue, token).await?;
    let known_prefix = domain_queue.prefix.clone();
    let cancelled = || RanchError::Cancelled;

    let focus = domain_queue.job(args.job_id.clone());
    let focus_key = keys.job_hash(&focus);

    // 1. Up to the root.
    let mut root = focus.clone();
    let mut first = true;
    for _ in 0..flow::MAX_ANCESTORS {
        if token.is_cancelled() {
            return Err(cancelled());
        }
        let probe = probe_flow_jobs(&mut conn, keys, std::slice::from_ref(&root))
            .await
            .map_err(redis_to_ranch_err)?
            .pop();
        let Some(probe) = probe else { break };
        if first && !probe.exists {
            return Err(RanchError::JobNotFound {
                queue: domain_queue.name.clone(),
                id: args.job_id.clone(),
            });
        }
        first = false;
        match probe
            .parent_key
            .as_deref()
            .and_then(|k| flow::parse_job_key(k, &known_prefix))
        {
            Some(parent) => root = parent,
            None => break,
        }
    }

    // 2. Down, one pipeline per level.
    let root_key = keys.job_hash(&root);
    let mut nodes: Vec<FlowNode> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut level: Vec<(JobRef, Option<String>)> = vec![(root, None)];
    seen.insert(root_key.clone());
    let mut truncated = false;
    while !level.is_empty() && !truncated {
        if token.is_cancelled() {
            return Err(cancelled());
        }
        let refs: Vec<JobRef> = level.iter().map(|(r, _)| r.clone()).collect();
        let probes = probe_flow_jobs(&mut conn, keys, &refs)
            .await
            .map_err(redis_to_ranch_err)?;
        let mut next: Vec<(JobRef, Option<String>)> = Vec::new();
        for ((jref, parent_key), probe) in level.into_iter().zip(probes) {
            if nodes.len() >= flow::MAX_NODES {
                truncated = true;
                break;
            }
            let key = keys.job_hash(&jref);
            for child_key in &probe.children {
                if seen.contains(child_key) {
                    continue; // a cycle, or a child listed twice
                }
                if let Some(child) = flow::parse_job_key(child_key, &known_prefix) {
                    seen.insert(child_key.clone());
                    next.push((child, Some(key.clone())));
                }
            }
            nodes.push(FlowNode {
                key,
                name: probe.name,
                job_ref: jref,
                parent_key,
                state: probe.state,
                timestamp: probe.timestamp,
                duration_ms: probe.duration_ms,
                failed_reason: probe.failed_reason,
                missing: !probe.exists,
                return_value: probe.return_value,
                blocks_parent: probe.state == JobState::Failed
                    && flow::failure_blocks_parent(probe.opts.as_deref()),
            });
        }
        if nodes.len() + next.len() > flow::MAX_NODES {
            next.truncate(flow::MAX_NODES.saturating_sub(nodes.len()));
            truncated = true;
            // One more pass for what still fits, then stop.
            let refs: Vec<JobRef> = next.iter().map(|(r, _)| r.clone()).collect();
            let probes = probe_flow_jobs(&mut conn, keys, &refs)
                .await
                .map_err(redis_to_ranch_err)?;
            for ((jref, parent_key), probe) in next.into_iter().zip(probes) {
                nodes.push(FlowNode {
                    key: keys.job_hash(&jref),
                    name: probe.name,
                    job_ref: jref,
                    parent_key,
                    state: probe.state,
                    timestamp: probe.timestamp,
                    duration_ms: probe.duration_ms,
                    failed_reason: probe.failed_reason,
                    missing: !probe.exists,
                    return_value: probe.return_value,
                    blocks_parent: probe.state == JobState::Failed
                        && flow::failure_blocks_parent(probe.opts.as_deref()),
                });
            }
            break;
        }
        level = next;
    }

    // A blocking child only blocks a parent that waits for it.
    let waiting: std::collections::HashSet<&str> = nodes
        .iter()
        .filter(|n| n.state == states::WAITING_CHILDREN)
        .map(|n| n.key.as_str())
        .collect();
    let blocked_children: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            n.blocks_parent && n.parent_key.as_deref().is_some_and(|p| waiting.contains(p))
        })
        .map(|(i, _)| i)
        .collect();
    let blocked = !blocked_children.is_empty();
    for (i, node) in nodes.iter_mut().enumerate() {
        node.blocks_parent = blocked_children.contains(&i);
    }
    Ok(FlowTree {
        nodes,
        root_key,
        focus_key,
        truncated,
        blocked,
    })
}

/// Flow roots of a queue, bounded: every parent waiting
/// for children (up to `DISCOVERY_WAITING_CHILDREN`) plus the most recently
/// finished jobs of the queue, per state. A root = has children, no parent.
pub(crate) async fn handle_list_flow_roots(
    state: &mut ActorState,
    queue_id: &QueueId,
    token: &CancellationToken,
) -> Result<FlowRoots, RanchError> {
    let mut conn = open_redis_for_actor(state, token).await?;
    let q = &QueueRef::parse(queue_id.as_str())?;
    let keys = resolve_keys_for_queue(state, &mut conn, q, token).await?;
    let recent = flow::DISCOVERY_RECENT_PER_STATE as i64;

    type Candidates = (Vec<String>, Vec<String>, Vec<String>, u64, u64);
    let mut candidates = redis::pipe();
    candidates
        .cmd("ZREVRANGE")
        .arg(keys.waiting_children(q))
        .arg(0)
        .arg(flow::DISCOVERY_WAITING_CHILDREN as i64 - 1)
        .cmd("ZREVRANGE")
        .arg(keys.completed(q))
        .arg(0)
        .arg(recent - 1)
        .cmd("ZREVRANGE")
        .arg(keys.failed(q))
        .arg(0)
        .arg(recent - 1)
        .cmd("ZCARD")
        .arg(keys.completed(q))
        .cmd("ZCARD")
        .arg(keys.failed(q));
    let (waiting_children, completed, failed, completed_total, failed_total): Candidates = tokio::select! {
        r = candidates.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::Cancelled),
    };
    let capped = completed_total > recent as u64 || failed_total > recent as u64;

    let mut ids: Vec<String> = Vec::new();
    for id in waiting_children.into_iter().chain(completed).chain(failed) {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let refs: Vec<JobRef> = ids.into_iter().map(|id| q.job(id)).collect();

    let mut roots: Vec<FlowRoot> = Vec::new();
    for chunk in refs.chunks(200) {
        if token.is_cancelled() {
            return Err(RanchError::Cancelled);
        }
        let probes = probe_flow_jobs(&mut conn, keys, chunk)
            .await
            .map_err(redis_to_ranch_err)?;
        for (jref, probe) in chunk.iter().zip(probes) {
            if probe.children.is_empty() || probe.parent_key.is_some() || !probe.exists {
                continue;
            }
            // What became of the children is their own state, not which of
            // the parent's side keys lists them: a child that failed without
            // `failParentOnFailure` stays in `:dependencies` and the parent
            // waits for it forever, which is exactly what this view is for.
            let child_refs: Vec<JobRef> = probe
                .children
                .iter()
                .take(flow::MAX_NODES)
                .filter_map(|k| flow::parse_job_key(k, &q.prefix))
                .collect();
            let child_probes = probe_flow_jobs(&mut conn, keys, &child_refs)
                .await
                .map_err(redis_to_ranch_err)?;
            let count =
                |wanted: JobState| child_probes.iter().filter(|c| c.state == wanted).count() as u64;
            // A processed child removed by `removeOnComplete` has no state
            // left; the parent's `processed` hash remembers it completed.
            let blocked = flow::is_blocked(
                probe.state,
                &child_probes
                    .iter()
                    .map(|c| (c.state, flow::failure_blocks_parent(c.opts.as_deref())))
                    .collect::<Vec<_>>(),
            );
            let completed = count(JobState::Completed).max(probe.completed);
            let failed = count(JobState::Failed).max(probe.failed);
            let total = probe.children.len() as u64;
            roots.push(FlowRoot {
                job_ref: jref.clone(),
                name: probe.name,
                state: probe.state,
                timestamp: probe.timestamp,
                children_total: total,
                children_pending: total.saturating_sub(completed + failed),
                children_completed: completed,
                children_failed: failed,
                blocked,
            });
        }
    }
    roots.sort_by_key(|r| std::cmp::Reverse(r.timestamp));

    Ok(FlowRoots {
        roots,
        scanned: refs.len() as u64,
        capped,
    })
}
