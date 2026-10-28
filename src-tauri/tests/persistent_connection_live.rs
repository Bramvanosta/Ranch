//! A connection actor keeps one Redis connection, and a slow
//! link stays reachable. Against Ranch's own Redis: `RANCH_LOCAL_REDIS=1`.

use std::sync::Arc;
use std::time::Duration;

use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{
    add_connection_inner, ping_connection_inner, AddConnectionArgs,
};
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

/// The connection count is Redis-wide: the tests of this file run one at a
/// time, so one does not count the other's connections.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Connections Redis has accepted since it started.
async fn connections_received(redis: &mut redis::aio::MultiplexedConnection) -> u64 {
    let info: String = redis::cmd("INFO")
        .arg("stats")
        .query_async(redis)
        .await
        .unwrap();
    info.lines()
        .find_map(|l| l.strip_prefix("total_connections_received:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap()
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn commands_share_one_redis_connection() {
    let _turn = ONE_AT_A_TIME.lock().await;
    let port = redis_target::port();
    let dir = tempfile::tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        args("persistent", port),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let registry = ConnectionRegistry::new();
    let mut probe = redis::Client::open(format!("redis://127.0.0.1:{port}"))
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();

    let before = connections_received(&mut probe).await;
    for _ in 0..5 {
        list_queues_inner(&conn.id, &common::backends(db.pool(), &kc, &registry))
            .await
            .unwrap();
        let ping = ping_connection_inner(&conn.id, &common::backends(db.pool(), &kc, &registry))
            .await
            .unwrap();
        assert!(ping.success, "{:?}", ping.error);
    }
    let opened = connections_received(&mut probe).await - before;

    // Before the connection was kept: one per command, plus one per queue for the counters.
    assert_eq!(opened, 1, "five list_queues and five pings opened {opened}");
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn a_slow_link_is_slow_not_unreachable() {
    let _turn = ONE_AT_A_TIME.lock().await;
    let port = redis_target::port();
    // 600 ms round trip: past redis-rs's 500 ms reply default, which made
    // every command fail before the connection was kept.
    let proxy = slow_proxy(port, Duration::from_millis(300)).await;
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

    let ping = ping_connection_inner(&conn.id, &common::backends(db.pool(), &kc, &registry))
        .await
        .unwrap();
    assert!(ping.success, "{:?}", ping.error);
    let latency = ping.latency_ms.unwrap();
    assert!(
        (550..1500).contains(&latency),
        "the latency is the PING round trip, not a handshake: {latency} ms"
    );

    let queues = list_queues_inner(&conn.id, &common::backends(db.pool(), &kc, &registry))
        .await
        .unwrap();
    assert!(!queues.is_empty(), "the seeded queues are listed");
}
