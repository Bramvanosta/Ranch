//! A connection actor keeps one Redis connection, and a slow
//! link stays reachable. Against Ranch's own Redis: `RANCH_LOCAL_REDIS=1`.

use std::sync::Arc;
use std::time::Duration;

use ranch_lib::commands::connections::{
    add_connection_inner, ping_connection_inner, AddConnectionArgs,
};
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
async fn commands_share_one_redis_connection() {
    let Some(port) = redis_port() else { return };
    let dir = tempfile::tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn KeychainAdapter> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(args("persistent", port), db.pool(), &kc)
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
        list_queues_inner(&conn.id, db.pool(), &kc, &registry)
            .await
            .unwrap();
        let ping = ping_connection_inner(&conn.id, db.pool(), &kc, Some(&registry))
            .await
            .unwrap();
        assert!(ping.success, "{:?}", ping.error);
    }
    let opened = connections_received(&mut probe).await - before;

    // Before the connection was kept: one per command, plus one per queue for the counters.
    assert_eq!(opened, 1, "five list_queues and five pings opened {opened}");
}

#[tokio::test]
async fn a_slow_link_is_slow_not_unreachable() {
    let Some(port) = redis_port() else { return };
    // 600 ms round trip: past redis-rs's 500 ms reply default, which made
    // every command fail before the connection was kept.
    let proxy = slow_proxy(port, Duration::from_millis(300)).await;
    let dir = tempfile::tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn KeychainAdapter> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(args("far", proxy), db.pool(), &kc)
        .await
        .unwrap();
    let registry = ConnectionRegistry::new();

    let ping = ping_connection_inner(&conn.id, db.pool(), &kc, Some(&registry))
        .await
        .unwrap();
    assert!(ping.success, "{:?}", ping.error);
    let latency = ping.latency_ms.unwrap();
    assert!(
        (550..1500).contains(&latency),
        "the latency is the PING round trip, not a handshake: {latency} ms"
    );

    let queues = list_queues_inner(&conn.id, db.pool(), &kc, &registry)
        .await
        .unwrap();
    assert!(!queues.is_empty(), "the seeded queues are listed");
}
