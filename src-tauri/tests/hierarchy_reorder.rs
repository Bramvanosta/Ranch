//! Integration tests for `reorder_groups_inner` + `reorder_environments_inner`.

mod common;
use common::repo;

use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::commands::connections::{
    add_connection_inner, list_connections_inner, AddConnectionArgs,
};
use ranch_lib::commands::hierarchy::{reorder_environments_inner, reorder_groups_inner};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::domain::errors::RanchError;
use sqlx::Row;
use tempfile::tempdir;

fn args(name: &str, color: &str) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.to_string(),
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
        environment: EnvironmentSelector::for_color(color),
    }
}

#[tokio::test]
async fn reorder_envs_changes_list_ordering() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    // Three connections in three different envs (auto-created in insertion order).
    let local = add_connection_inner(
        args("a", "env-local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let staging = add_connection_inner(
        args("b", "env-staging"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let production = add_connection_inner(
        args("c", "env-production"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    let group_id = local.group_id.clone(); // all share the Default group

    // Initial ordering (by env display_order = 0 for all → tiebreak on conn created_at).
    // We don't assert the initial order; we just reorder and assert the new one.

    // Reorder: production first, then staging, then local.
    reorder_environments_inner(
        &group_id,
        &[
            production.environment_id.clone(),
            staging.environment_id.clone(),
            local.environment_id.clone(),
        ],
        &repo(db.pool()),
    )
    .await
    .unwrap();

    let listed = list_connections_inner(&repo(db.pool())).await.unwrap();
    let names: Vec<&str> = listed.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["c", "b", "a"]);
}

#[tokio::test]
async fn reorder_envs_persists_display_order() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let local = add_connection_inner(
        args("a", "env-local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let staging = add_connection_inner(
        args("b", "env-staging"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    let group_id = local.group_id.clone();
    reorder_environments_inner(
        &group_id,
        &[staging.environment_id.clone(), local.environment_id.clone()],
        &repo(db.pool()),
    )
    .await
    .unwrap();

    // Verify the column directly.
    let row: (i64,) = sqlx::query_as("SELECT display_order FROM environments WHERE id = ?")
        .bind(&staging.environment_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(row.0, 0);

    let row: (i64,) = sqlx::query_as("SELECT display_order FROM environments WHERE id = ?")
        .bind(&local.environment_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(row.0, 1);
}

#[tokio::test]
async fn reorder_envs_rejects_extra_id() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        args("a", "env-local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let group_id = conn.group_id.clone();

    let err = reorder_environments_inner(
        &group_id,
        &[conn.environment_id.clone(), "phantom-id".to_string()],
        &repo(db.pool()),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, RanchError::InvalidInput { .. }));
}

#[tokio::test]
async fn reorder_envs_rejects_missing_id() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    add_connection_inner(
        args("a", "env-local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let conn_b = add_connection_inner(
        args("b", "env-staging"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    let err = reorder_environments_inner(
        &conn_b.group_id,
        std::slice::from_ref(&conn_b.environment_id),
        &repo(
            // missing the env-local id
            db.pool(),
        ),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, RanchError::InvalidInput { .. }));
}

#[tokio::test]
async fn reorder_envs_rejects_duplicates() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        args("a", "env-local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let group_id = conn.group_id.clone();

    let err = reorder_environments_inner(
        &group_id,
        &[conn.environment_id.clone(), conn.environment_id.clone()],
        &repo(db.pool()),
    )
    .await
    .unwrap_err();
    match err {
        RanchError::InvalidInput { reason } => assert!(reason.contains("duplicates")),
        other => panic!("expected InvalidInput on duplicates, got {other:?}"),
    }
}

#[tokio::test]
async fn reorder_groups_with_single_group_is_a_no_op_pass() {
    // We only auto-create the "Default" group. Reordering a single group
    // should still succeed (no-op equivalent).
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        args("a", "env-local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    reorder_groups_inner(std::slice::from_ref(&conn.group_id), &repo(db.pool()))
        .await
        .unwrap();

    // display_order set to 0 for the single group.
    let order: i64 = sqlx::query("SELECT display_order FROM groups WHERE id = ?")
        .bind(&conn.group_id)
        .fetch_one(db.pool())
        .await
        .unwrap()
        .get(0);
    assert_eq!(order, 0);
}
