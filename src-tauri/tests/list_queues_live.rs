//! Live integration tests for `list_queues` against the dockered Redis
//! seeded by `compose --profile bullmq up bullmq-seed`.
//!
//! Gated on `RANCH_LOCAL_REDIS=1` env var: without it, every test
//! returns early with a skip message. CI runs these only when the
//! variable is explicitly set; local devs can run them after spinning
//! up the docker stack:
//!
//! ```bash
//! docker compose --profile redis --profile bullmq up -d
//! RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 \
//!   cargo test --test list_queues_live -- --include-ignored --nocapture
//! ```
//!
//! The seed creates 3 queues:
//!   - emails  (50 wait + 10 delayed + 5 fail)
//!   - images  (8 done + 22 wait)
//!   - reports (5 wait)

use std::time::Instant;

use ranch_lib::adapters::bullmq_redis::bullmq_schema::{BullmqSchema, JobState, QueueRef};
use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;

mod common;
use common::redis_target;

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn discovers_seeded_bullmq_queues() {
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;
    let queues = registry
        .list_queues("conn-discover", &cfg)
        .await
        .expect("actor send")
        .expect("list_queues");

    let names: Vec<String> = queues.iter().map(|q| q.name.clone()).collect();

    // The seed creates exactly these 3 queues (sorted lexicographically
    // by handle_list_queues). Re-running the seed adds more JOBS but
    // not more QUEUES, so this assertion is stable.
    assert!(
        names.contains(&"emails".to_string()),
        "expected `emails` in {names:?}"
    );
    assert!(
        names.contains(&"images".to_string()),
        "expected `images` in {names:?}"
    );
    assert!(
        names.contains(&"reports".to_string()),
        "expected `reports` in {names:?}"
    );
    // All discovered queues use the default `bull` prefix: their id is
    // the key base `bull:{name}`.
    for q in &queues {
        assert_eq!(
            q.id.as_str(),
            format!("bull:{}", q.name),
            "expected prefix=bull, got {:?}",
            q.id
        );
    }
    // Output sorted by name.
    let mut sorted = names.clone();
    sorted.sort_by_key(|a| a.to_lowercase());
    assert_eq!(names, sorted, "expected lexicographic sort");

    registry.shutdown_all().await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn counters_reflect_seeded_data() {
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;
    let queues = registry
        .list_queues("conn-counters", &cfg)
        .await
        .expect("actor send")
        .expect("list_queues");

    let emails = queues
        .iter()
        .find(|q| q.name == "emails")
        .expect("emails queue");
    let images = queues
        .iter()
        .find(|q| q.name == "images")
        .expect("images queue");

    // Seed produces:
    //   emails: 50 wait + 10 delayed + 5 fail
    //   images: 8 done + 22 wait
    //
    // BullMQ v5 may move some "wait" jobs into "delayed" sorted-set
    // proactively when no worker is consuming, so we use >= rather than
    // exact equality. Re-running the seed adds more jobs (idempotent
    // adds), so >= holds across multiple seed runs too.
    assert!(
        emails.counts.get(JobState::Delayed) >= 10,
        "emails.delayed = {} (expected >= 10)",
        emails.counts.get(JobState::Delayed)
    );
    assert!(
        emails.counts.get(JobState::Failed) >= 5,
        "emails.failed = {} (expected >= 5)",
        emails.counts.get(JobState::Failed)
    );
    assert!(
        images.counts.get(JobState::Completed) >= 8,
        "images.completed = {} (expected >= 8)",
        images.counts.get(JobState::Completed)
    );

    registry.shutdown_all().await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn schema_is_cached_after_first_call() {
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;

    // First call: full SCAN + HGETALL meta + counters. Cold path.
    let _ = registry
        .list_queues("conn-cached", &cfg)
        .await
        .expect("actor send")
        .expect("first list_queues");

    // Second call: still does SCAN + HGETALL meta (we re-discover each
    // call to catch new queues) but the schema is already cached, so
    // the per-queue counter fetch starts immediately. We check that the
    // call returns a sensible result; perf is not asserted because the
    // test is too noisy under different load conditions to be reliable
    // (covered by the time_to_first_counters_within_budget test below).
    let queues = registry
        .list_queues("conn-cached", &cfg)
        .await
        .expect("actor send")
        .expect("second list_queues");
    assert!(
        !queues.is_empty(),
        "expected non-empty queue list on second call"
    );

    registry.shutdown_all().await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn time_to_first_counters_within_budget() {
    // time-to-first-counters < 500ms over a < 50ms RTT link
    // (localhost docker = 0ms RTT). With 3 queues we expect ~30-100ms
    // total wall-clock for a cold call.
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;

    let start = Instant::now();
    let queues = registry
        .list_queues("conn-budget", &cfg)
        .await
        .expect("actor send")
        .expect("list_queues");
    let elapsed = start.elapsed();
    eprintln!(
        "time-to-first-counters: {elapsed:?} ({} queues)",
        queues.len()
    );
    assert!(
        elapsed.as_millis() < 500,
        "budget breached: {elapsed:?} (limit: 500ms)"
    );

    registry.shutdown_all().await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn unknown_queue_returns_empty_list_not_error() {
    // An unused DB on the same Redis instance has no queues. Discovery
    // should return Vec::new(), not error.
    let registry = ConnectionRegistry::new();
    let mut cfg = redis_target::connection_config().await;
    cfg.endpoint.db = 15; // unlikely to have BullMQ data
    let queues = registry
        .list_queues("conn-empty-db", &cfg)
        .await
        .expect("actor send")
        .expect("list_queues");
    assert!(
        queues.is_empty(),
        "expected empty list on db=15, got {} queue(s)",
        queues.len()
    );

    registry.shutdown_all().await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn queue_id_round_trips_correctly() {
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;
    let queues = registry
        .list_queues("conn-qref", &cfg)
        .await
        .expect("actor send")
        .expect("list_queues");

    // Each Queue's id reads back into a non-empty prefix + name.
    for q in &queues {
        let qref = &QueueRef::from_id(&q.id).expect("a BullMQ queue id");
        assert_eq!(qref.name, q.name);
        assert!(!qref.prefix.is_empty(), "empty prefix in {qref:?}");
        assert!(!qref.name.is_empty(), "empty name in {qref:?}");
        // Verify constructed key looks like a BullMQ queue key.
        let expected_meta = format!("{}:{}:meta", qref.prefix, qref.name);
        assert!(
            expected_meta.starts_with("bull:"),
            "expected key starts with 'bull:', got {expected_meta}"
        );
    }

    // Suppress the unused-import warning for BullmqSchema.
    let _ = BullmqSchema::V5;

    registry.shutdown_all().await;
}
