//! Live tests for server-side job search / sort / progress.
//!
//!   docker compose --profile redis up -d
//!   RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test list_jobs_query_live -- --include-ignored
//!
//! Self-seeded: each test writes its own queue under the `ranch92` prefix
//! (BullMQ v5 layout) and removes it afterwards, so nothing depends on, or
//! disturbs, the shared `bull:*` seed the other live suites mutate.

use ranch_lib::adapters::bullmq_redis::bullmq_schema::{JobState, QueueRef};
use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::domain::jobs::{JobPageQuery, JobSort, JobSortField};

mod common;
use common::redis_target;

const PREFIX: &str = "ranch92";

async fn raw_conn() -> redis::aio::MultiplexedConnection {
    let client = redis::Client::open(format!(
        "redis://{}:{}/0",
        redis_target::host(),
        redis_target::port()
    ))
    .expect("redis url");
    client
        .get_multiplexed_async_connection()
        .await
        .expect("redis connection")
}

/// 120 failed jobs, ids 1..=120, finished at `1_000_000 + id` (so the
/// natural newest-first order is 120 → 1). Job 7, far beyond the first
/// page, is the only `needle-export`; every 10th job failed on a timeout;
/// the duration grows with the id except job 50, which never started.
async fn seed_failed_queue(queue: &str) {
    let mut conn = raw_conn().await;
    let mut pipe = redis::pipe();
    pipe.cmd("HSET")
        .arg(format!("{PREFIX}:{queue}:meta"))
        .arg("version")
        .arg("bullmq:5.34.0");
    for id in 1..=120i64 {
        let name = if id == 7 {
            "needle-export"
        } else {
            "send-email"
        };
        let reason = if id % 10 == 0 {
            "connect ETIMEDOUT 10.0.0.4:5432"
        } else {
            "Validation failed"
        };
        let queued = 900_000 + id;
        let mut hset = redis::cmd("HSET");
        hset.arg(format!("{PREFIX}:{queue}:{id}"))
            .arg("name")
            .arg(name)
            .arg("data")
            .arg("{}")
            .arg("opts")
            .arg("{}")
            .arg("timestamp")
            .arg(queued)
            .arg("failedReason")
            .arg(reason)
            .arg("attemptsMade")
            .arg(1)
            .arg("progress")
            .arg(if id == 7 {
                r#"{"step":"upload","pct":40}"#
            } else {
                "0"
            });
        if id != 50 {
            hset.arg("processedOn")
                .arg(1_000_000)
                .arg("finishedOn")
                .arg(1_000_000 + id);
        }
        pipe.add_command(hset);
        pipe.cmd("ZADD")
            .arg(format!("{PREFIX}:{queue}:failed"))
            .arg(1_000_000 + id)
            .arg(id);
    }
    let _: () = pipe.query_async(&mut conn).await.expect("seed");
}

async fn cleanup(queue: &str) {
    let mut conn = raw_conn().await;
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg(format!("{PREFIX}:{queue}:*"))
        .query_async(&mut conn)
        .await
        .expect("keys");
    if !keys.is_empty() {
        let _: () = redis::cmd("DEL")
            .arg(keys)
            .query_async(&mut conn)
            .await
            .expect("del");
    }
}

fn args(queue: &str, offset: u64, limit: u64) -> JobPageQuery {
    JobPageQuery {
        queue_id: QueueRef {
            prefix: PREFIX.into(),
            name: queue.into(),
        }
        .id(),
        state: JobState::Failed,
        offset,
        limit,
        newest_first: true,
        search: None,
        reason: None,
        sort: None,
    }
}

fn ids(out: &ranch_lib::domain::jobs::JobPage) -> Vec<String> {
    out.jobs.iter().map(|j| j.job_ref.id.clone()).collect()
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn search_finds_a_job_that_is_not_on_the_loaded_page() {
    let queue = "search";
    cleanup(queue).await;
    seed_failed_queue(queue).await;
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;

    // The plain path: page 1 holds jobs 120..=71: the needle is not in it.
    let plain = registry
        .list_jobs("q92-search", &cfg, args(queue, 0, 50))
        .await
        .expect("actor send")
        .expect("list_jobs");
    assert_eq!(plain.total, 120);
    assert_eq!(plain.scanned, None, "no query = no scan");
    assert!(!plain.scan_capped);
    assert!(plain.has_more);
    assert!(!ids(&plain).contains(&"7".to_string()));

    // The same request with a search reaches it.
    let mut a = args(queue, 0, 50);
    a.search = Some("NEEDLE".into());
    let found = registry
        .list_jobs("q92-search", &cfg, a)
        .await
        .expect("actor send")
        .expect("list_jobs");
    assert_eq!(ids(&found), vec!["7"]);
    assert_eq!(found.total, 1, "total = matching jobs, not the state size");
    assert_eq!(found.scanned, Some(120));
    assert!(!found.has_more);
    assert!(!found.scan_capped);
    let needle = &found.jobs[0];
    assert_eq!(needle.state, JobState::Failed);
    assert_eq!(
        needle.progress.as_deref(),
        Some(r#"{"step":"upload","pct":40}"#),
        "a reported progress is decoded"
    );

    // Search also matches the id; BullMQ's initial `0` progress is dropped.
    let mut a = args(queue, 0, 50);
    a.search = Some("119".into());
    let by_id = registry
        .list_jobs("q92-search", &cfg, a)
        .await
        .expect("actor send")
        .expect("list_jobs");
    assert_eq!(ids(&by_id), vec!["119"]);
    assert_eq!(by_id.jobs[0].progress, None);

    registry.shutdown_all().await;
    cleanup(queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn reason_filter_pages_over_the_matching_set() {
    let queue = "reason";
    cleanup(queue).await;
    seed_failed_queue(queue).await;
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;

    let mut a = args(queue, 0, 5);
    a.reason = Some("etimedout".into());
    let page1 = registry
        .list_jobs("q92-reason", &cfg, a.clone())
        .await
        .expect("actor send")
        .expect("list_jobs");
    assert_eq!(page1.total, 12, "every 10th of 120 jobs timed out");
    assert_eq!(ids(&page1), vec!["120", "110", "100", "90", "80"]);
    assert!(page1.has_more);

    a.offset = 10;
    let page3 = registry
        .list_jobs("q92-reason", &cfg, a)
        .await
        .expect("actor send")
        .expect("list_jobs");
    assert_eq!(ids(&page3), vec!["20", "10"]);
    assert!(!page3.has_more);

    registry.shutdown_all().await;
    cleanup(queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn sorts_the_whole_state_by_duration_and_queue_time() {
    let queue = "sort";
    cleanup(queue).await;
    seed_failed_queue(queue).await;
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;

    let mut a = args(queue, 0, 3);
    a.sort = Some(JobSort {
        field: JobSortField::Duration,
        descending: false,
    });
    let fastest = registry
        .list_jobs("q92-sort", &cfg, a.clone())
        .await
        .expect("actor send")
        .expect("list_jobs");
    assert_eq!(ids(&fastest), vec!["1", "2", "3"]);
    assert_eq!(fastest.total, 120);

    // Job 50 never started: no duration, so it closes the list both ways.
    a.offset = 117;
    let tail = registry
        .list_jobs("q92-sort", &cfg, a.clone())
        .await
        .expect("actor send")
        .expect("list_jobs");
    assert_eq!(ids(&tail), vec!["119", "120", "50"]);

    a.offset = 0;
    a.sort = Some(JobSort {
        field: JobSortField::QueuedAt,
        descending: false,
    });
    let oldest = registry
        .list_jobs("q92-sort", &cfg, a)
        .await
        .expect("actor send")
        .expect("list_jobs");
    assert_eq!(ids(&oldest), vec!["1", "2", "3"]);

    registry.shutdown_all().await;
    cleanup(queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn a_newer_query_supersedes_the_one_still_scanning() {
    let queue = "supersede";
    cleanup(queue).await;
    seed_failed_queue(queue).await;
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;

    // The user typed "need", then "needle" before the first scan finished.
    // Both are requested back to back: the first one has round trips to make
    // before its first chunk, by which time the second has taken its ticket.
    let mut first = args(queue, 0, 50);
    first.search = Some("need".into());
    let mut second = args(queue, 0, 50);
    second.search = Some("needle".into());
    let (older, newer) = tokio::join!(
        registry.list_jobs("q92-supersede", &cfg, first),
        registry.list_jobs("q92-supersede", &cfg, second),
    );
    let older = older.expect("actor send");
    assert!(
        matches!(
            older,
            Err(ranch_lib::domain::errors::RanchError::Superseded)
        ),
        "the older scan must let go, got {older:?}"
    );
    let newer = newer.expect("actor send").expect("newer query");
    assert_eq!(ids(&newer), vec!["7"]);

    // A plain page read takes no ticket: it is never superseded, and a
    // query issued before it is not cancelled by it.
    let mut query = args(queue, 0, 50);
    query.search = Some("needle".into());
    let (query, plain) = tokio::join!(
        registry.list_jobs("q92-supersede", &cfg, query),
        registry.list_jobs("q92-supersede", &cfg, args(queue, 0, 50)),
    );
    assert_eq!(ids(&query.expect("actor send").expect("query")), vec!["7"]);
    assert_eq!(plain.expect("actor send").expect("plain").jobs.len(), 50);

    registry.shutdown_all().await;
    cleanup(queue).await;
}

/// A job exactly as BullMQ's `storeJob.lua` leaves it: no `atm` until an
/// attempt ends. A decoder that required that field would list the waiting
/// / delayed / active jobs of a real queue as empty, next to non-zero
/// counters.
#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn lists_jobs_that_have_never_run() {
    let queue = "q92-never-ran";
    cleanup(queue).await;
    let mut conn = raw_conn().await;
    let mut pipe = redis::pipe();
    pipe.cmd("HSET")
        .arg(format!("{PREFIX}:{queue}:meta"))
        .arg("version")
        .arg("bullmq:5.34.0");
    for id in ["1", "2"] {
        pipe.cmd("HSET")
            .arg(format!("{PREFIX}:{queue}:{id}"))
            .arg("name")
            .arg("send-email")
            .arg("data")
            .arg(r#"{"to":"alice@example.com"}"#)
            .arg("opts")
            .arg(r#"{"attempts":3}"#)
            .arg("timestamp")
            .arg("1700000000000")
            .arg("delay")
            .arg(if id == "2" { "60000" } else { "0" })
            .arg("priority")
            .arg("0");
    }
    pipe.cmd("RPUSH")
        .arg(format!("{PREFIX}:{queue}:wait"))
        .arg("1");
    pipe.cmd("ZADD")
        .arg(format!("{PREFIX}:{queue}:delayed"))
        .arg(1_700_000_060_000_i64 * 4096)
        .arg("2");
    let _: () = pipe.query_async(&mut conn).await.expect("seed");

    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;
    for (state, id) in [(JobState::Waiting, "1"), (JobState::Delayed, "2")] {
        let mut a = args(queue, 0, 50);
        a.state = state;
        let out = registry
            .list_jobs("q92-never-ran", &cfg, a)
            .await
            .expect("actor")
            .expect("list_jobs");
        assert_eq!(ids(&out), vec![id.to_string()], "{state:?}");
        assert_eq!(out.jobs[0].attempts_made, 0);
    }
    cleanup(queue).await;
}

/// Recent activity: running jobs first, then the latest finished ones,
/// completed and failed merged, across queues when no queue is given.
#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn recent_jobs_merge_running_completed_and_failed() {
    use ranch_lib::domain::jobs::RecentJobsQuery;
    // Far in the future, so the shared `bull:*` seed never outranks them.
    const T: i64 = 4_102_444_800_000; // 2100-01-01
    let (qa, qb) = ("q92-recent-a", "q92-recent-b");
    cleanup(qa).await;
    cleanup(qb).await;
    let mut conn = raw_conn().await;
    let mut pipe = redis::pipe();
    let mut add = |queue: &str, id: &str, processed: i64, finished: Option<i64>| {
        let mut hset = redis::cmd("HSET");
        hset.arg(format!("{PREFIX}:{queue}:{id}"))
            .arg("name")
            .arg("job")
            .arg("data")
            .arg("{}")
            .arg("opts")
            .arg("{}")
            .arg("timestamp")
            .arg(T - 10_000)
            .arg("processedOn")
            .arg(processed);
        if let Some(f) = finished {
            hset.arg("finishedOn").arg(f).arg("atm").arg(1);
        }
        pipe.add_command(hset);
    };
    add(qa, "done-1", T, Some(T + 100));
    add(qa, "failed-1", T, Some(T + 300));
    add(qa, "running-1", T + 400, None);
    add(qb, "done-2", T, Some(T + 200));
    for q in [qa, qb] {
        pipe.cmd("HSET")
            .arg(format!("{PREFIX}:{q}:meta"))
            .arg("version")
            .arg("bullmq:5.34.0");
    }
    pipe.cmd("ZADD")
        .arg(format!("{PREFIX}:{qa}:completed"))
        .arg(T + 100)
        .arg("done-1");
    pipe.cmd("ZADD")
        .arg(format!("{PREFIX}:{qa}:failed"))
        .arg(T + 300)
        .arg("failed-1");
    pipe.cmd("LPUSH")
        .arg(format!("{PREFIX}:{qa}:active"))
        .arg("running-1");
    pipe.cmd("ZADD")
        .arg(format!("{PREFIX}:{qb}:completed"))
        .arg(T + 200)
        .arg("done-2");
    let _: () = pipe.query_async(&mut conn).await.expect("seed");

    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;

    // One queue.
    let one = registry
        .list_recent_jobs(
            "q92-recent",
            &cfg,
            RecentJobsQuery {
                queue_id: Some(
                    QueueRef {
                        prefix: PREFIX.into(),
                        name: qa.into(),
                    }
                    .id(),
                ),
                limit: 10,
            },
        )
        .await
        .expect("actor")
        .expect("list_recent_jobs");
    let one_ids: Vec<&str> = one.jobs.iter().map(|j| j.job_ref.id.as_str()).collect();
    assert_eq!(one_ids, ["running-1", "failed-1", "done-1"]);
    assert_eq!(one.jobs[0].state, JobState::Active);
    assert_eq!(one.jobs[1].state, JobState::Failed);
    assert_eq!(one.jobs[2].state, JobState::Completed);

    // The whole connection: both queues, merged by time.
    let all = registry
        .list_recent_jobs(
            "q92-recent",
            &cfg,
            RecentJobsQuery {
                queue_id: None,
                limit: 50,
            },
        )
        .await
        .expect("actor")
        .expect("list_recent_jobs");
    let mine: Vec<String> = all
        .jobs
        .iter()
        .filter(|j| {
            j.job_ref
                .queue_id
                .as_str()
                .starts_with(&format!("{PREFIX}:q92-recent"))
        })
        .map(|j| j.job_ref.id.clone())
        .collect();
    assert_eq!(mine, ["running-1", "failed-1", "done-2", "done-1"]);

    cleanup(qa).await;
    cleanup(qb).await;
}
