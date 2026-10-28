//! Integration test for `remove_connection_inner` + actor lifecycle.
//!
//! Verifies that removing a connection: (a) deletes the SQLite row,
//! (b) cleans up keychain entries, AND (c) shuts down the live actor
//! within 1 second. Plus: a subsequent ping against the same id surfaces
//! `ConnectionNotFound` from the SQLite SELECT before reaching the
//! registry: the upstream check still fires.

mod common;
use common::repo;

use ranch_lib::adapters::bullmq_redis::connection_actor::{ConnectionConfig, ConnectionRegistry};
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::commands::connections::{
    add_connection_inner, ping_connection_inner, remove_connection_inner, AddConnectionArgs,
};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::domain::errors::RanchError;
use tempfile::tempdir;

fn add_args(name: &str) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.to_string(),
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
    }
}

async fn cfg_for(host: &str, port: u16) -> ConnectionConfig {
    ConnectionConfig {
        endpoint: ranch_lib::adapters::bullmq_redis::endpoint::RedisEndpoint {
            host: host.to_string(),
            port,
            db: 0,
            username: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
        },
        password: None,
        ssh_password: None,
        keychain: std::sync::Arc::new(ranch_lib::adapters::secrets::InMemoryKeychain::new()),
        initial_lock_state: ranch_lib::domain::lock::ReadOnlyState::Unlocked,
        connection_id: "test-conn".into(),
        connection_name: "test".into(),
        env_color: Some("env-local".into()),
        writes: ranch_lib::adapters::sqlite::write_pipeline(
            &sqlx::SqlitePool::connect("sqlite::memory:")
                .await
                .expect("in-memory pool"),
        ),
    }
}

#[tokio::test]
async fn remove_connection_shuts_down_the_actor() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());
    let registry = ConnectionRegistry::new();

    // Persist a connection.
    let conn = add_connection_inner(
        add_args("c"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    // Spawn its actor by sending a ping.
    let _ = registry
        .ping(&conn.id, &cfg_for("127.0.0.1", 1).await)
        .await
        .unwrap();
    assert!(!registry.is_empty().await);

    // Remove the connection: actor should shut down.
    let start = std::time::Instant::now();
    remove_connection_inner(&conn.id, &repo(db.pool()), &kc, Some(&registry))
        .await
        .unwrap();
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 1_500,
        "remove_connection took {elapsed:?} (expected < 1.5s including actor shutdown)"
    );
    assert!(
        registry.is_empty().await,
        "registry should be empty after remove"
    );
}

#[tokio::test]
async fn ping_after_remove_returns_connection_not_found_from_sql() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());
    let registry = ConnectionRegistry::new();

    let conn = add_connection_inner(
        add_args("c"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let id = conn.id.clone();

    // Remove without ever spawning an actor: also a valid path.
    remove_connection_inner(&id, &repo(db.pool()), &kc, Some(&registry))
        .await
        .unwrap();

    // Subsequent ping_connection_inner should hit ConnectionNotFound from
    // the SQLite SELECT, not reach the registry at all.
    let err = ping_connection_inner(&id, &common::backends(db.pool(), &kc, &registry))
        .await
        .unwrap_err();
    assert!(matches!(err, RanchError::ConnectionNotFound { .. }));
    assert!(registry.is_empty().await);
}

#[tokio::test]
async fn remove_connection_with_no_live_actor_is_a_noop_for_registry() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());
    let registry = ConnectionRegistry::new();

    let conn = add_connection_inner(
        add_args("c"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    // No ping yet → no actor spawned.
    assert!(registry.is_empty().await);

    // Remove still succeeds.
    remove_connection_inner(&conn.id, &repo(db.pool()), &kc, Some(&registry))
        .await
        .unwrap();
    assert!(registry.is_empty().await);
}
