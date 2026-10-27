//! Live test for the overview's live events.
//!
//!   docker compose --profile redis --profile bullmq up -d
//!   RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test events_live
//!
//! The seed leaves `added` / `completed` / `failed` events in the `emails`
//! and `images` streams; the test adds one event of its own with XADD.

use ranch_lib::commands::queues::ReadQueueEventsArgs;
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

fn skip() -> bool {
    if std::env::var("RANCH_LOCAL_REDIS").is_err() {
        eprintln!("skipping: set RANCH_LOCAL_REDIS=1 (see the module doc)");
        return true;
    }
    false
}

#[tokio::test]
async fn reads_the_last_events_then_only_what_is_new() {
    if skip() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;
    let read = |cursors| ReadQueueEventsArgs {
        connection_id: "test-conn".into(),
        cursors,
        limit: None,
    };

    let first = registry
        .read_queue_events("ev", &cfg, read(Vec::new()))
        .await
        .expect("actor send")
        .expect("events");
    assert!(!first.events.is_empty(), "the seed leaves events behind");
    assert!(first.events.windows(2).all(|w| w[0].at_ms >= w[1].at_ms));
    assert!(first.cursors.iter().any(|c| c.queue_name == "emails"));
    // Completed / failed events carry no name: it is read from the job.
    assert!(first
        .events
        .iter()
        .any(|e| e.event != "added" && e.job_name.is_some()));

    let again = registry
        .read_queue_events("ev", &cfg, read(first.cursors.clone()))
        .await
        .unwrap()
        .unwrap();
    assert!(
        again.events.is_empty(),
        "nothing new since: {:?}",
        again.events
    );
    assert!(!again.sampled);

    // Something happens on `emails`.
    let client = redis::Client::open(format!("redis://127.0.0.1:{}/", cfg.port)).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let _: String = redis::cmd("XADD")
        .arg("bull:emails:events")
        .arg("*")
        .arg("event")
        .arg("failed")
        .arg("jobId")
        .arg("live-test")
        .arg("failedReason")
        .arg("boom")
        .query_async(&mut conn)
        .await
        .unwrap();
    let news = registry
        .read_queue_events("ev", &cfg, read(again.cursors.clone()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(news.events.len(), 1, "{:?}", news.events);
    let e = &news.events[0];
    assert_eq!(
        (e.queue_name.as_str(), e.event.as_str()),
        ("emails", "failed")
    );
    assert_eq!(e.failed_reason.as_deref(), Some("boom"));
    assert_eq!(e.job_name, None, "no such job: no name, the event stays");
}
