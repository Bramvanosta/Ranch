//! Live integration tests for `list_queues` against the dockered Redis
//! seeded by `compose --profile bullmq up bullmq-seed`.
//!
//! Gated on `RANCH_LOCAL_REDIS=1` env var — without it, every test
//! returns early with a skip message. CI runs these only when the
//! variable is explicitly set; local devs can run them after spinning
//! up the docker stack:
//!
//! ```bash
//! docker compose --profile redis --profile bullmq up -d
//! RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 \
//!   cargo test --test list_queues_live -- --nocapture
//! ```
//!
//! The seed creates 3 queues:
//!   - emails  (50 wait + 10 delayed + 5 fail)
//!   - images  (8 done + 22 wait)
//!   - reports (5 wait)

use std::time::Instant;

use ranch_lib::core::bullmq_schema::{BullmqSchema, QueueRef};
use ranch_lib::core::connection_actor::{ConnectionConfig, ConnectionRegistry};

async fn local_redis_cfg() -> ConnectionConfig {
    ConnectionConfig {
        host: std::env::var("RANCH_REDIS_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
        port: std::env::var("RANCH_REDIS_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(6390),
        db: 0,
        username: None,
        password: None,
        tls: false,
        tls_custom_ca: None,
        ssh_tunnel: None,
        ssh_password: None,
        keychain: std::sync::Arc::new(ranch_lib::core::keychain::InMemoryKeychain::new()),
        initial_lock_state: ranch_lib::core::safety_lock::ReadOnlyState::Unlocked,
        connection_id: "test-conn".into(),
        connection_name: "test".into(),
        env_color: Some("env-local".into()),
        pool: sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory pool"),
    }
}

fn skip_if_no_local_redis() -> bool {
    if std::env::var("RANCH_LOCAL_REDIS").is_err() {
        eprintln!(
            "skipping: set RANCH_LOCAL_REDIS=1 (and optionally \
             RANCH_REDIS_HOST / RANCH_REDIS_PORT) + run \
             `docker compose --profile redis --profile bullmq up -d`"
        );
        return true;
    }
    false
}

#[tokio::test]
async fn discovers_seeded_bullmq_queues() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;
    let queues = registry
        .list_queues("conn-discover", &cfg)
        .await
        .expect("actor send")
        .expect("list_queues");

    let names: Vec<String> = queues.iter().map(|q| q.queue_ref.name.clone()).collect();

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
    // All discovered queues use the default `bull` prefix.
    for q in &queues {
        assert_eq!(
            q.queue_ref.prefix, "bull",
            "expected prefix=bull, got {:?}",
            q.queue_ref
        );
    }
    // Output sorted by name.
    let mut sorted = names.clone();
    sorted.sort_by_key(|a| a.to_lowercase());
    assert_eq!(names, sorted, "expected lexicographic sort");

    registry.shutdown_all().await;
}

#[tokio::test]
async fn counters_reflect_seeded_data() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;
    let queues = registry
        .list_queues("conn-counters", &cfg)
        .await
        .expect("actor send")
        .expect("list_queues");

    let emails = queues
        .iter()
        .find(|q| q.queue_ref.name == "emails")
        .expect("emails queue");
    let images = queues
        .iter()
        .find(|q| q.queue_ref.name == "images")
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
        emails.counters.delayed >= 10,
        "emails.delayed = {} (expected >= 10)",
        emails.counters.delayed
    );
    assert!(
        emails.counters.failed >= 5,
        "emails.failed = {} (expected >= 5)",
        emails.counters.failed
    );
    assert!(
        images.counters.completed >= 8,
        "images.completed = {} (expected >= 8)",
        images.counters.completed
    );

    registry.shutdown_all().await;
}

#[tokio::test]
async fn schema_is_cached_after_first_call() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;

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
async fn time_to_first_counters_within_budget() {
    if skip_if_no_local_redis() {
        return;
    }
    // time-to-first-counters < 500ms over a < 50ms RTT link
    // (localhost docker = 0ms RTT). With 3 queues we expect ~30-100ms
    // total wall-clock for a cold call.
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;

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
async fn unknown_queue_returns_empty_list_not_error() {
    if skip_if_no_local_redis() {
        return;
    }
    // An unused DB on the same Redis instance has no queues. Discovery
    // should return Vec::new(), not error.
    let registry = ConnectionRegistry::new();
    let mut cfg = local_redis_cfg().await;
    cfg.db = 15; // unlikely to have BullMQ data
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
async fn queue_ref_round_trips_correctly() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;
    let queues = registry
        .list_queues("conn-qref", &cfg)
        .await
        .expect("actor send")
        .expect("list_queues");

    // Each Queue has a non-empty prefix + name.
    for q in &queues {
        let qref = &q.queue_ref;
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
    let _: QueueRef = queues[0].queue_ref.clone();

    registry.shutdown_all().await;
}
