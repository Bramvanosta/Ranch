//! The queue registry: once warm, listing every queue's
//! counters is one round trip; a deleted queue leaves the list. Against
//! Ranch's own Redis: `RANCH_LOCAL_REDIS=1`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::queues::list_queues_inner;
use ranch_lib::core::connection_actor::ConnectionRegistry;
use ranch_lib::core::db::Db;
use ranch_lib::core::keychain::{InMemoryKeychain, KeychainAdapter};

mod common;
use common::slow_proxy;

fn redis_port() -> Option<u16> {
    if std::env::var("RANCH_LOCAL_REDIS").ok().as_deref() != Some("1") {
        return None;
    }
    Some(
        std::env::var("RANCH_REDIS_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(6390),
    )
}

fn args(name: &str, port: u16) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.into(),
        host: "127.0.0.1".into(),
        port,
        db: 0,
        username: None,
        password: None,
        tls: false,
        tls_custom_ca: None,
        ssh_tunnel: None,
        ssh_password: None,
        group: GroupSelector::named("Default"),
        environment: EnvironmentSelector::for_color("env-local"),
    }
}

#[tokio::test]
async fn a_warm_queue_list_is_one_round_trip() {
    let Some(port) = redis_port() else { return };
    let rtt = Duration::from_millis(600);
    let proxy = slow_proxy(port, rtt / 2).await;
    let dir = tempfile::tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn KeychainAdapter> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(args("far", proxy), db.pool(), &kc)
        .await
        .unwrap();
    let registry = ConnectionRegistry::new();

    // Cold: connect, SCAN, the meta hashes, the counters.
    let first = list_queues_inner(&conn.id, db.pool(), &kc, &registry)
        .await
        .unwrap();
    assert!(!first.is_empty(), "the seeded queues are listed");

    let started = Instant::now();
    let warm = list_queues_inner(&conn.id, db.pool(), &kc, &registry)
        .await
        .unwrap();
    let took = started.elapsed();
    assert_eq!(warm.len(), first.len());
    // Before the queue registry was cached: a SCAN round trip, one HGETALL per queue, then the
    // counters — at least 2 + Q round trips.
    assert!(
        took < rtt * 2,
        "{} queues listed in {took:?} on a {rtt:?} link",
        warm.len()
    );
}

#[tokio::test]
async fn a_deleted_queue_leaves_the_list() {
    let Some(port) = redis_port() else { return };
    let dir = tempfile::tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn KeychainAdapter> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(args("registry", port), db.pool(), &kc)
        .await
        .unwrap();
    let registry = ConnectionRegistry::new();
    let mut redis = redis::Client::open(format!("redis://127.0.0.1:{port}"))
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();

    let name = format!("registry-{}", chrono::Utc::now().timestamp_millis());
    let _: () = redis::cmd("HSET")
        .arg(format!("bull:{name}:meta"))
        .arg("version")
        .arg("bullmq:5.30.0")
        .query_async(&mut redis)
        .await
        .unwrap();
    let _: () = redis::cmd("LPUSH")
        .arg(format!("bull:{name}:wait"))
        .arg("1")
        .query_async(&mut redis)
        .await
        .unwrap();

    let listed = list_queues_inner(&conn.id, db.pool(), &kc, &registry)
        .await
        .unwrap();
    let mine = listed.iter().find(|q| q.queue_ref.name == name);
    assert_eq!(mine.map(|q| q.counters.waiting), Some(1));

    let _: () = redis::cmd("DEL")
        .arg(format!("bull:{name}:meta"))
        .arg(format!("bull:{name}:wait"))
        .query_async(&mut redis)
        .await
        .unwrap();
    // The registry still holds it; the counter read sees the meta gone.
    let listed = list_queues_inner(&conn.id, db.pool(), &kc, &registry)
        .await
        .unwrap();
    assert!(
        listed.iter().all(|q| q.queue_ref.name != name),
        "an obliterated queue is not listed with zero counters"
    );
}
