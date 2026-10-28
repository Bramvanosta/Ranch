//! Integration test for `add_connection_inner`.
//!
//! Validates the contract:
//!   - A successful add inserts a connection row with `keychain_ref` matching
//!     the new connection's id, and stores the password under that id in the
//!     keychain.
//!   - Adding two connections with the same env_color reuses the same
//!     environment row (auto-creates only on first use).
//!   - When the keychain refuses a `set`, the entire transaction rolls back
//!     and no connection row is left behind.
//!
//! Uses an InMemoryKeychain so the test is hermetic: no OS keychain access
//! and no Linux libsecret dependency.

mod common;
use common::repo;

use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::domain::errors::RanchError;
use sqlx::Row;
use std::sync::Mutex;
use tempfile::tempdir;

fn args_with(name: &str, color: &str, password: Option<&str>) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.to_string(),
        backend: "bullmq-redis".into(),
        endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
            host: "redis.example.com".to_string(),
            port: 6379,
            db: 0,
            username: Some("default".to_string()),
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
        }
        .json(),
        password: password.map(str::to_string),
        ssh_password: None,
        group: GroupSelector::named("Default"),
        environment: EnvironmentSelector::for_color(color),
    }
}

#[tokio::test]
async fn happy_path_persists_row_and_credential() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");
    let keychain: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        args_with("Local Redis", "env-local", Some("hunter2")),
        &repo(db.pool()),
        &keychain,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .expect("add_connection_inner");

    assert_eq!(conn.name, "Local Redis");
    assert_eq!(conn.env_color, "env-local");
    assert_eq!(conn.env_name, "Local"); // derived from color token
    assert_eq!(conn.group_name, "Default");
    assert_eq!(conn.backend, "bullmq-redis");
    let endpoint = ranch_lib::commands::connections::BullmqRedisEndpoint::read(&conn.endpoint)
        .expect("a Redis endpoint");
    assert_eq!(endpoint.host, "redis.example.com");
    assert!(!endpoint.tls);

    // Row exists with keychain_ref = id.
    let row = sqlx::query("SELECT keychain_ref FROM connections WHERE id = ?")
        .bind(&conn.id)
        .fetch_one(db.pool())
        .await
        .expect("fetch row");
    let keychain_ref: String = row.get("keychain_ref");
    assert_eq!(keychain_ref, conn.id);

    // Credential lives in the keychain under the connection id.
    let stored = keychain.get(&conn.id).expect("keychain.get");
    assert_eq!(stored.as_deref(), Some("hunter2"));
}

#[tokio::test]
async fn no_password_skips_keychain_write_but_persists_row() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");
    let keychain: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        args_with("Auth-less Redis", "env-staging", None),
        &repo(db.pool()),
        &keychain,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .expect("add_connection_inner");

    // Row exists.
    let count: i64 = sqlx::query("SELECT COUNT(*) FROM connections WHERE id = ?")
        .bind(&conn.id)
        .fetch_one(db.pool())
        .await
        .expect("count")
        .get(0);
    assert_eq!(count, 1);

    // Keychain has no entry for this id.
    assert_eq!(keychain.get(&conn.id).expect("get"), None);
}

#[tokio::test]
async fn second_connection_same_color_reuses_environment() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");
    let keychain: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let a = add_connection_inner(
        args_with("first", "env-production", Some("p1")),
        &repo(db.pool()),
        &keychain,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .expect("first add");

    let b = add_connection_inner(
        args_with("second", "env-production", Some("p2")),
        &repo(db.pool()),
        &keychain,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .expect("second add");

    assert_eq!(a.environment_id, b.environment_id);
    assert_eq!(a.group_id, b.group_id);

    // One group, one env, two connections.
    let group_count: i64 = sqlx::query("SELECT COUNT(*) FROM groups")
        .fetch_one(db.pool())
        .await
        .expect("count groups")
        .get(0);
    assert_eq!(group_count, 1);
    let env_count: i64 = sqlx::query("SELECT COUNT(*) FROM environments")
        .fetch_one(db.pool())
        .await
        .expect("count envs")
        .get(0);
    assert_eq!(env_count, 1);
}

#[tokio::test]
async fn different_colors_create_separate_environments() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");
    let keychain: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let a = add_connection_inner(
        args_with("local", "env-local", None),
        &repo(db.pool()),
        &keychain,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .expect("local");

    let b = add_connection_inner(
        args_with("prod", "env-production", None),
        &repo(db.pool()),
        &keychain,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .expect("prod");

    assert_ne!(a.environment_id, b.environment_id);
    assert_eq!(a.group_id, b.group_id); // both in Default group

    let env_count: i64 = sqlx::query("SELECT COUNT(*) FROM environments")
        .fetch_one(db.pool())
        .await
        .expect("count")
        .get(0);
    assert_eq!(env_count, 2);
}

#[tokio::test]
async fn keychain_failure_rolls_back_db_insert() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");
    let failing_keychain: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(AlwaysFailingKeychain::new());

    let result = add_connection_inner(
        args_with("doomed", "env-local", Some("would-fail")),
        &repo(db.pool()),
        &failing_keychain,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await;

    assert!(result.is_err(), "expected keychain failure to propagate");
    match result.unwrap_err() {
        RanchError::KeychainUnavailable { .. } => {}
        other => panic!("expected KeychainUnavailable, got {other:?}"),
    }

    // No connection row should have been committed.
    let count: i64 = sqlx::query("SELECT COUNT(*) FROM connections")
        .fetch_one(db.pool())
        .await
        .expect("count")
        .get(0);
    assert_eq!(
        count, 0,
        "connection row must roll back when keychain fails"
    );

    // The auto-created group/env from upsert helpers also rolls back since
    // they share the transaction.
    let group_count: i64 = sqlx::query("SELECT COUNT(*) FROM groups")
        .fetch_one(db.pool())
        .await
        .expect("count groups")
        .get(0);
    assert_eq!(group_count, 0, "default group must roll back too");
}

/// Test double for the keychain that returns `KeychainUnavailable` from `set`.
struct AlwaysFailingKeychain {
    sets_attempted: Mutex<u32>,
}

impl AlwaysFailingKeychain {
    fn new() -> Self {
        Self {
            sets_attempted: Mutex::new(0),
        }
    }
}

impl SecretStore for AlwaysFailingKeychain {
    fn set(&self, _connection_id: &str, _secret: &str) -> Result<(), RanchError> {
        *self.sets_attempted.lock().unwrap() += 1;
        Err(RanchError::KeychainUnavailable {
            reason: "simulated test failure".to_string(),
            doc_url: "https://example.test/keychain".to_string(),
        })
    }

    fn get(&self, _connection_id: &str) -> Result<Option<String>, RanchError> {
        Ok(None)
    }

    fn delete(&self, _connection_id: &str) -> Result<(), RanchError> {
        Ok(())
    }
}
