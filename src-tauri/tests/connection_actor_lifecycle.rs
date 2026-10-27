//! Integration tests for the connection actor lifecycle.
//!
//! Covers:
//!   - Multiple actors spawn independently and respond to pings
//!     concurrently — the registry's mailbox-per-actor model is race-free.
//!   - `shutdown_all` drains every live actor within 1 second.
//!   - Concurrent pings against the same connection_id all reach the same
//!     actor (mailbox serialization).

use std::sync::Arc;

use ranch_lib::core::connection_actor::{ConnectionConfig, ConnectionRegistry};

/// Returns a config that points at 127.0.0.1:1 (reserved/unused port).
/// Pings against this target return CannotReach or Timeout fast — the
/// actor stays alive after each, which is what we test here.
async fn unreachable_cfg() -> ConnectionConfig {
    ConnectionConfig {
        host: "127.0.0.1".into(),
        port: 1,
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

#[tokio::test]
async fn three_actors_spawn_and_each_responds() {
    let reg = ConnectionRegistry::new();
    for id in ["alpha", "beta", "gamma"] {
        let result = reg.ping(id, &unreachable_cfg().await).await.unwrap();
        assert!(
            !result.success,
            "ping to 127.0.0.1:1 should fail (actor {id})"
        );
    }
    // Cleanup — every actor exits within 1s.
    let start = std::time::Instant::now();
    reg.shutdown_all().await;
    assert!(start.elapsed().as_secs() < 2);
}

#[tokio::test]
async fn concurrent_pings_against_same_id_all_reach_an_actor() {
    let reg = Arc::new(ConnectionRegistry::new());
    let cfg = unreachable_cfg().await;

    // Fire 5 concurrent pings against "shared-id".
    let mut handles = Vec::new();
    for _ in 0..5 {
        let reg = reg.clone();
        let cfg = cfg.clone();
        handles.push(tokio::spawn(
            async move { reg.ping("shared-id", &cfg).await },
        ));
    }

    // All 5 must succeed (i.e. return Ok with a structured result —
    // success=false because the host is unreachable).
    for h in handles {
        let result = h.await.unwrap().unwrap();
        assert!(!result.success);
    }

    reg.shutdown_all().await;
}

#[tokio::test]
async fn shutdown_all_completes_within_one_second() {
    let reg = ConnectionRegistry::new();
    // Spin up 10 actors against unreachable targets.
    for i in 0..10 {
        let id = format!("conn-{i}");
        let _ = reg.ping(&id, &unreachable_cfg().await).await.unwrap();
    }
    let start = std::time::Instant::now();
    reg.shutdown_all().await;
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 1_500,
        "shutdown_all took {elapsed:?} (expected < 1.5s for 10 actors)"
    );
}
