//! Live test for the overview's live events.
//!
//!   docker compose --profile redis --profile bullmq up -d
//!   RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test events_live -- --include-ignored
//!
//! The seed leaves `added` / `completed` / `failed` events in the `emails`
//! and `images` streams; the test adds one event of its own with XADD.

use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::domain::events::EventsQuery;

mod common;
use common::redis_target;

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn reads_the_last_events_then_only_what_is_new() {
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;
    let read = |cursors| EventsQuery {
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
    assert!(first
        .cursors
        .iter()
        .any(|c| c.queue_id.as_str() == "bull:emails"));
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
    let client = redis::Client::open(format!("redis://127.0.0.1:{}/", cfg.endpoint.port)).unwrap();
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
