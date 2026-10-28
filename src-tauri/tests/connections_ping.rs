//! Integration test for `ping_connection_inner`.
//!
//! Always-runnable contract: pinging a saved-but-unreachable connection
//! returns success=false with CannotReach (or Timeout depending on platform);
//! pinging a non-existent connection id returns ConnectionNotFound.
//!
//! No real-Redis test in CI; same gating as `test_connection`. Real-Redis
//! validation is part of the manual macOS smoke step.

mod common;
use common::repo;

use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::commands::connections::{
    add_connection_inner, ping_connection_inner, AddConnectionArgs, TestConnectionError,
};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::domain::errors::RanchError;
use tempfile::tempdir;

#[tokio::test]
async fn ping_dead_port_returns_unreachable_or_timeout() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    // Insert a connection pointing at 127.0.0.1:1 (reserved/refused).
    let conn = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "dead".to_string(),
            backend: "bullmq-redis".into(),
            endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
                host: "127.0.0.1".to_string(),
                port: 1,
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
        },
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .expect("add");

    let result = ping_connection_inner(
        &conn.id,
        &common::backends(
            db.pool(),
            &kc,
            &ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry::new(),
        ),
    )
    .await
    .expect("ping (outer Result is Ok even on unreachable)");

    assert!(!result.success, "expected ping to report unreachable");
    assert!(result.latency_ms.is_none());
    assert!(result.error.is_some());

    // Either CannotReach or Timeout is acceptable; both signal "user can't
    // reach the host" depending on platform-level connection-refused semantics.
    match result.error.unwrap() {
        TestConnectionError::CannotReach { .. } | TestConnectionError::Timeout => {}
        other => panic!("expected CannotReach or Timeout, got {other:?}"),
    }
}

#[tokio::test]
async fn ping_unknown_id_returns_connection_not_found() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let err = ping_connection_inner(
        "nonexistent-id",
        &common::backends(
            db.pool(),
            &kc,
            &ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry::new(),
        ),
    )
    .await
    .expect_err("expected ConnectionNotFound");

    match err {
        RanchError::ConnectionNotFound { id } => assert_eq!(id, "nonexistent-id"),
        other => panic!("expected ConnectionNotFound, got {other:?}"),
    }
}
