//! The queue registry: once warm, listing every queue's
//! counters is one round trip; a deleted queue leaves the list. Against
//! Ranch's own Redis: `RANCH_LOCAL_REDIS=1`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::queues::list_queues_inner;

mod common;
use common::{redis_target, repo, slow_proxy};

fn args(name: &str, port: u16) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.into(),
        backend: "bullmq-redis".into(),
        endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
            host: "127.0.0.1".into(),
            port,
            db: 0,
            username: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
        }
        .json(),
        password: None,
        ssh_password: None,
        group: GroupSelector::named("Default"),
        environment: EnvironmentSelector::for_color("env-local"),
    }
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn a_warm_queue_list_is_one_round_trip() {
    let port = redis_target::port();
    let rtt = Duration::from_millis(600);
    let proxy = slow_proxy(port, rtt / 2).await;
    let dir = tempfile::tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        args("far", proxy),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let registry = ConnectionRegistry::new();

    // Cold: connect, SCAN, the meta hashes, the counters.
    let first = list_queues_inner(&conn.id, &common::backends(db.pool(), &kc, &registry))
        .await
        .unwrap();
    assert!(!first.is_empty(), "the seeded queues are listed");

    let started = Instant::now();
    let warm = list_queues_inner(&conn.id, &common::backends(db.pool(), &kc, &registry))
        .await
        .unwrap();
    let took = started.elapsed();
    assert_eq!(warm.len(), first.len());
    // Before the queue registry was cached: a SCAN round trip, one HGETALL per queue, then the
    // counters: at least 2 + Q round trips.
    assert!(
        took < rtt * 2,
        "{} queues listed in {took:?} on a {rtt:?} link",
        warm.len()
    );
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn a_deleted_queue_leaves_the_list() {
    let port = redis_target::port();
    let dir = tempfile::tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        args("registry", port),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
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

    let listed = list_queues_inner(&conn.id, &common::backends(db.pool(), &kc, &registry))
        .await
        .unwrap();
    let mine = listed.iter().find(|q| q.name == name);
    assert_eq!(mine.map(|q| q.counts.get("waiting")), Some(1));

    let _: () = redis::cmd("DEL")
        .arg(format!("bull:{name}:meta"))
        .arg(format!("bull:{name}:wait"))
        .query_async(&mut redis)
        .await
        .unwrap();
    // The registry still holds it; the counter read sees the meta gone.
    let listed = list_queues_inner(&conn.id, &common::backends(db.pool(), &kc, &registry))
        .await
        .unwrap();
    assert!(
        listed.iter().all(|q| q.name != name),
        "an obliterated queue is not listed with zero counters"
    );
}
