//! Integration tests for `remove_connection_inner`.
//!
//! Validates:
//!   - Happy path with password: row + keychain both gone.
//!   - Auth-less connection: row gone (no keychain entry to delete, no error).
//!   - Unknown id → ConnectionNotFound.
//!   - KeychainUnavailable on delete: row still removed, warning logged
//!     (verified by the test double recording the call).

mod common;
use common::repo;

use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{
    add_connection_inner, list_connections_inner, remove_connection_inner, AddConnectionArgs,
};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::domain::errors::RanchError;
use sqlx::Row;
use std::sync::Mutex;
use tempfile::tempdir;

fn args(password: Option<&str>) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: "to-remove".to_string(),
        backend: "bullmq-redis".into(),
        endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
            host: "h".to_string(),
            port: 6379,
            db: 0,
            username: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
        }
        .json(),
        password: password.map(str::to_string),
        ssh_password: None,
        group: GroupSelector::named("Default"),
        environment: EnvironmentSelector::for_color("env-local"),
    }
}

#[tokio::test]
async fn remove_with_password_clears_row_and_keychain() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        args(Some("secret")),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    remove_connection_inner(&conn.id, &repo(db.pool()), &kc, None)
        .await
        .unwrap();

    let listed = list_connections_inner(&repo(db.pool())).await.unwrap();
    assert!(listed.iter().all(|c| c.id != conn.id));
    assert_eq!(kc.get(&conn.id).unwrap(), None);
}

#[tokio::test]
async fn remove_auth_less_connection_succeeds() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        args(None),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    // Sanity: no keychain entry for an auth-less connection.
    assert_eq!(kc.get(&conn.id).unwrap(), None);

    remove_connection_inner(&conn.id, &repo(db.pool()), &kc, None)
        .await
        .unwrap();

    let listed = list_connections_inner(&repo(db.pool())).await.unwrap();
    assert!(listed.iter().all(|c| c.id != conn.id));
}

#[tokio::test]
async fn remove_unknown_id_returns_connection_not_found() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let err = remove_connection_inner("nope", &repo(db.pool()), &kc, None)
        .await
        .unwrap_err();
    assert!(matches!(err, RanchError::ConnectionNotFound { .. }));
}

#[tokio::test]
async fn keychain_unavailable_during_delete_still_removes_row() {
    // Design: leaking a stale credential is preferable to leaving
    // a row pointing at a missing one. Verify the row is gone even when the
    // keychain reports unavailable.
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    // Use the regular keychain to seed the row.
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        args(Some("secret")),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    // Now delete with a keychain that always reports unavailable.
    let unavailable_kc_raw = UnavailableOnDeleteKeychain::default();
    let deletes_counter = unavailable_kc_raw.deletes_attempted.clone();
    let unavailable_kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(unavailable_kc_raw);
    remove_connection_inner(&conn.id, &repo(db.pool()), &unavailable_kc, None)
        .await
        .expect("remove should succeed even when keychain reports unavailable");

    // Row removed despite keychain failure.
    let count: i64 = sqlx::query("SELECT COUNT(*) FROM connections WHERE id = ?")
        .bind(&conn.id)
        .fetch_one(db.pool())
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 0);

    // `remove_connection` attempts FOUR keychain
    // deletes: `{id}` (Redis password) + `{id}::ssh` (SSH password) +
    // `{id}::ssh::hostkey` (SSH host-key pin) + the jump host's pin. All
    // four are attempted even when the first reports KeychainUnavailable.
    assert_eq!(*deletes_counter.lock().unwrap(), 4);
}

/// Test double whose `delete` always returns `KeychainUnavailable`.
/// `deletes_attempted` is `Arc<Mutex<_>>` so the test can hold a
/// counter handle alongside the keychain (which we move into an Arc
/// to satisfy the new `remove_connection_inner` signature).
#[derive(Default)]
struct UnavailableOnDeleteKeychain {
    deletes_attempted: std::sync::Arc<Mutex<u32>>,
}

impl SecretStore for UnavailableOnDeleteKeychain {
    fn set(&self, _id: &str, _secret: &str) -> Result<(), RanchError> {
        Ok(())
    }
    fn get(&self, _id: &str) -> Result<Option<String>, RanchError> {
        Ok(Some("dummy".to_string()))
    }
    fn delete(&self, _id: &str) -> Result<(), RanchError> {
        *self.deletes_attempted.lock().unwrap() += 1;
        Err(RanchError::KeychainUnavailable {
            reason: "simulated daemon down".to_string(),
            doc_url: "https://example.test".to_string(),
        })
    }
}
