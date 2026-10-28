//! A followed connection reads its queues' event streams and
//! says which queues moved. Against Ranch's own Redis: `RANCH_LOCAL_REDIS=1`.

use std::sync::Arc;
use std::time::Duration;

use ranch_lib::adapters::bullmq_redis::connection_actor::{
    ConnectionConfig, ConnectionRegistry, EventSink,
};
use ranch_lib::domain::events::QueueEventsNotice;

mod common;
use common::redis_target;

async fn cfg(port: u16) -> ConnectionConfig {
    ConnectionConfig {
        endpoint: ranch_lib::adapters::bullmq_redis::endpoint::RedisEndpoint {
            host: "127.0.0.1".into(),
            port,
            db: 0,
            username: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
        },
        password: None,
        ssh_password: None,
        keychain: Arc::new(ranch_lib::adapters::secrets::InMemoryKeychain::new()),
        initial_lock_state: ranch_lib::domain::lock::ReadOnlyState::Unlocked,
        connection_id: "follow-conn".into(),
        connection_name: "follow".into(),
        env_color: Some("env-local".into()),
        writes: ranch_lib::adapters::sqlite::write_pipeline(
            &sqlx::SqlitePool::connect("sqlite::memory:")
                .await
                .expect("in-memory pool"),
        ),
    }
}

async fn xadd(redis: &mut redis::aio::MultiplexedConnection, queue: &str, event: &str) {
    let _: String = redis::cmd("XADD")
        .arg(format!("bull:{queue}:events"))
        .arg("*")
        .arg("event")
        .arg(event)
        .arg("jobId")
        .arg("1")
        .query_async(redis)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn a_followed_connection_says_which_queues_moved() {
    let port = redis_target::port();
    let mut redis = redis::Client::open(format!("redis://127.0.0.1:{port}"))
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let queue = format!("follow-{}", chrono::Utc::now().timestamp_millis());
    let _: () = redis::cmd("HSET")
        .arg(format!("bull:{queue}:meta"))
        .arg("version")
        .arg("bullmq:5.30.0")
        .query_async(&mut redis)
        .await
        .unwrap();

    let registry = ConnectionRegistry::new();
    let config = cfg(port).await;
    // The follower reads the queues the registry knows.
    registry
        .list_queues("follow-conn", &config)
        .await
        .unwrap()
        .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<QueueEventsNotice>();
    let sink: EventSink = Arc::new(move |notice| {
        let _ = tx.send(notice);
    });
    let following = registry
        .follow_events("follow-conn", &config, true, Some(sink))
        .await
        .unwrap();
    assert!(following);

    // Written after following began: must be seen, even before the first read.
    xadd(&mut redis, &queue, "added").await;
    xadd(&mut redis, &queue, "completed").await;

    let notice = loop {
        let notice = tokio::time::timeout(Duration::from_secs(8), rx.recv())
            .await
            .expect("a queue:events notice within the block time")
            .expect("sink open");
        if notice
            .queues
            .iter()
            .any(|q| q.queue_id.as_str() == format!("bull:{queue}"))
        {
            break notice;
        }
    };
    let mine = notice
        .queues
        .iter()
        .find(|q| q.queue_id.as_str() == format!("bull:{queue}"))
        .unwrap();
    assert_eq!(notice.connection_id, "follow-conn");
    assert!(mine.events.contains(&"added".to_string()), "{mine:?}");

    // The last window stops following: no more notices.
    let following = registry
        .follow_events("follow-conn", &config, false, None)
        .await
        .unwrap();
    assert!(!following);
    tokio::time::sleep(Duration::from_millis(200)).await;
    while rx.try_recv().is_ok() {}
    xadd(&mut redis, &queue, "added").await;
    assert!(
        tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .map(|n| n.is_none())
            .unwrap_or(true),
        "a stopped follower sends nothing"
    );

    let _: () = redis::cmd("DEL")
        .arg(format!("bull:{queue}:meta"))
        .arg(format!("bull:{queue}:events"))
        .query_async(&mut redis)
        .await
        .unwrap();
}
