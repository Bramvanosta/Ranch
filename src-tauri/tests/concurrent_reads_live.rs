//! Reads on one connection run concurrently: a ping is answered
//! while a slow read is still going. Against Ranch's own Redis:
//! `RANCH_LOCAL_REDIS=1`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ranch_lib::commands::connections::{
    add_connection_inner, ping_connection_inner, AddConnectionArgs,
};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::queues::{get_topology_inner, list_queues_inner, GetTopologyArgs};
use ranch_lib::core::connection_actor::ConnectionRegistry;
use ranch_lib::core::db::Db;
use ranch_lib::core::keychain::{InMemoryKeychain, KeychainAdapter};

mod common;
use common::slow_proxy;

#[tokio::test]
async fn a_ping_does_not_wait_for_a_slow_read() {
    if std::env::var("RANCH_LOCAL_REDIS").ok().as_deref() != Some("1") {
        return;
    }
    let port: u16 = std::env::var("RANCH_REDIS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6390);
    let rtt = Duration::from_millis(800);
    let proxy = slow_proxy(port, rtt / 2).await;
    let dir = tempfile::tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn KeychainAdapter> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "concurrent".into(),
            host: "127.0.0.1".into(),
            port: proxy,
            db: 0,
            username: None,
            password: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
            ssh_password: None,
            group: GroupSelector::named("Default"),
            environment: EnvironmentSelector::for_color("env-local"),
        },
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    let registry = ConnectionRegistry::new();
    // Warm: the connection is open and the queues are known.
    list_queues_inner(&conn.id, db.pool(), &kc, &registry)
        .await
        .unwrap();

    let started = Instant::now();
    // The Map reads every queue in turn: several round trips.
    let slow = async {
        get_topology_inner(
            GetTopologyArgs {
                connection_id: conn.id.clone(),
            },
            db.pool(),
            &kc,
            &registry,
        )
        .await
        .unwrap();
        started.elapsed()
    };
    let ping = async {
        // Sent after the slow read is in the mailbox.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let sent = Instant::now();
        let result = ping_connection_inner(&conn.id, db.pool(), &kc, Some(&registry))
            .await
            .unwrap();
        assert!(result.success, "{:?}", result.error);
        sent.elapsed()
    };
    let (slow_took, ping_took) = tokio::join!(slow, ping);

    assert!(
        slow_took > rtt * 3,
        "the slow read is slow ({slow_took:?}), or the test proves nothing"
    );
    // Before reads ran side by side, the ping waited for the whole slow read.
    assert!(
        ping_took < rtt * 2,
        "a ping took {ping_took:?} behind a {slow_took:?} read on a {rtt:?} link"
    );
}
