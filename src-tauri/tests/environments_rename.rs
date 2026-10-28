//! Integration tests for `rename_environment_inner`.
//!
//! Hermetic: uses tempdir SQLite + InMemoryKeychain. Verifies:
//!   - Happy path: insert a connection (auto-creates an env), rename the env,
//!     `list_connections_inner` reflects the new label.
//!   - Empty / whitespace-only label → `InvalidInput`.
//!   - Unknown environment id → `EnvironmentNotFound`.

mod common;
use common::repo;

use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::commands::connections::{
    add_connection_inner, list_connections_inner, rename_environment_inner, AddConnectionArgs,
};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::domain::errors::RanchError;
use tempfile::tempdir;

fn args() -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: "any".to_string(),
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
        password: None,
        ssh_password: None,
        group: GroupSelector::named("Default"),
        environment: EnvironmentSelector::for_color("env-local"),
    }
}

#[tokio::test]
async fn happy_path_renames_label_and_list_reflects_it() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        args(),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .expect("add");
    // Default env label mirrors the color-derived
    // name (e.g. "Local" for `env-local`) rather than the generic
    // "Environment" placeholder.
    assert_eq!(conn.env_label, "Local");

    rename_environment_inner(&conn.environment_id, "  Local Stack  ", &repo(db.pool()))
        .await
        .expect("rename");

    let listed = list_connections_inner(&repo(db.pool()))
        .await
        .expect("list");
    let row = listed
        .iter()
        .find(|c| c.id == conn.id)
        .expect("conn in list");
    // Label is trimmed by the backend.
    assert_eq!(row.env_label, "Local Stack");
}

#[tokio::test]
async fn empty_label_rejected_with_invalid_input() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        args(),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .expect("add");

    let err = rename_environment_inner(&conn.environment_id, "   ", &repo(db.pool()))
        .await
        .expect_err("expected InvalidInput");

    match err {
        RanchError::InvalidInput { reason } => assert!(reason.to_lowercase().contains("empty")),
        other => panic!("expected InvalidInput, got {other:?}"),
    }

    // Original label is preserved on rejection (still the color-
    // derived default from the polish fix).
    let listed = list_connections_inner(&repo(db.pool()))
        .await
        .expect("list");
    let row = listed.iter().find(|c| c.id == conn.id).expect("conn");
    assert_eq!(row.env_label, "Local");
}

#[tokio::test]
async fn unknown_id_returns_environment_not_found() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");

    let err = rename_environment_inner("nope", "Whatever", &repo(db.pool()))
        .await
        .expect_err("expected EnvironmentNotFound");

    match err {
        RanchError::EnvironmentNotFound { id } => assert_eq!(id, "nope"),
        other => panic!("expected EnvironmentNotFound, got {other:?}"),
    }
}
