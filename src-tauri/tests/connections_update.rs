//! Integration tests for `update_connection_inner`.
//!
//! Hermetic — uses tempdir SQLite + InMemoryKeychain. Verifies:
//!   - Renaming the connection updates the row but doesn't touch keychain.
//!   - Changing env_color moves the connection to the new env (auto-created).
//!   - `password: None` leaves the existing keychain entry untouched.
//!   - `password: Some("new")` overwrites the keychain entry.
//!   - `password: Some("")` is rejected as InvalidInput.
//!   - Updating a non-existent id returns ConnectionNotFound.

use ranch_lib::commands::connections::{
    add_connection_inner, list_connections_inner, update_connection_inner, AddConnectionArgs,
    UpdateConnectionArgs,
};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::core::db::Db;
use ranch_lib::core::errors::RanchError;
use ranch_lib::core::keychain::InMemoryKeychain;
use tempfile::tempdir;

fn add_args(name: &str, password: Option<&str>, color: &str) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.to_string(),
        host: "h".to_string(),
        port: 6379,
        db: 0,
        username: None,
        password: password.map(str::to_string),
        tls: false,
        tls_custom_ca: None,
        ssh_tunnel: None,
        ssh_password: None,
        group: GroupSelector::named("Default"),
        environment: EnvironmentSelector::for_color(color),
    }
}

fn update_args(id: &str, name: &str, color: &str, password: Option<&str>) -> UpdateConnectionArgs {
    UpdateConnectionArgs {
        locked: None,
        id: id.to_string(),
        name: name.to_string(),
        host: "h".to_string(),
        port: 6379,
        db: 0,
        username: None,
        password: password.map(str::to_string),
        tls: false,
        tls_custom_ca: None,
        ssh_tunnel: None,
        ssh_tunnel_clear: false,
        ssh_password: None,
        group: GroupSelector::named("Default"),
        environment: EnvironmentSelector::for_color(color),
    }
}

#[tokio::test]
async fn rename_only_updates_row_and_leaves_keychain_alone() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        add_args("orig", Some("secret1"), "env-local"),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();

    update_connection_inner(
        update_args(&conn.id, "renamed", "env-local", None),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();

    let listed = list_connections_inner(db.pool()).await.unwrap();
    let row = listed.iter().find(|c| c.id == conn.id).unwrap();
    assert_eq!(row.name, "renamed");

    // Keychain entry untouched.
    assert_eq!(kc.get(&conn.id).unwrap().as_deref(), Some("secret1"));
}

#[tokio::test]
async fn changing_env_color_moves_to_new_env() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(add_args("c", None, "env-local"), db.pool(), &kc)
        .await
        .unwrap();
    let original_env_id = conn.environment_id.clone();

    update_connection_inner(
        update_args(&conn.id, "c", "env-production", None),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();

    let listed = list_connections_inner(db.pool()).await.unwrap();
    let row = listed.iter().find(|c| c.id == conn.id).unwrap();
    assert_eq!(row.env_color, "env-production");
    assert_ne!(
        row.environment_id, original_env_id,
        "should have moved to a different env"
    );
}

#[tokio::test]
async fn password_some_new_overwrites_keychain() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(add_args("c", Some("old"), "env-local"), db.pool(), &kc)
        .await
        .unwrap();
    update_connection_inner(
        update_args(&conn.id, "c", "env-local", Some("new")),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();

    assert_eq!(kc.get(&conn.id).unwrap().as_deref(), Some("new"));
}

#[tokio::test]
async fn password_some_empty_string_is_rejected() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(add_args("c", Some("old"), "env-local"), db.pool(), &kc)
        .await
        .unwrap();
    let err = update_connection_inner(
        update_args(&conn.id, "c", "env-local", Some("")),
        db.pool(),
        &kc,
    )
    .await
    .unwrap_err();
    match err {
        RanchError::InvalidInput { .. } => {}
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    // Keychain unchanged.
    assert_eq!(kc.get(&conn.id).unwrap().as_deref(), Some("old"));
}

#[tokio::test]
async fn unknown_id_returns_connection_not_found() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let err = update_connection_inner(
        update_args("does-not-exist", "x", "env-local", None),
        db.pool(),
        &kc,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, RanchError::ConnectionNotFound { .. }));
}
