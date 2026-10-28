//! Integration tests for group commands.
//!
//! Covers: migration default, list (incl. empty groups + counts), create
//! (ordering, trimming, validation, case-insensitive uniqueness), rename,
//! collapse persistence, delete-empty, delete guarded by `GroupNotEmpty`,
//! and the confirmed cascade with keychain cleanup.

mod common;
use common::repo;

use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{
    add_connection_inner, list_connections_inner, AddConnectionArgs,
};
use ranch_lib::commands::groups::{
    create_group_inner, delete_group_inner, list_groups_inner, rename_group_inner,
    set_group_collapsed_inner, GROUP_NAME_MAX_CHARS,
};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::domain::errors::RanchError;
use std::sync::Arc;
use tempfile::tempdir;

fn keychain() -> Arc<dyn SecretStore> {
    Arc::new(InMemoryKeychain::new())
}

fn add_args(name: &str, password: Option<&str>) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.to_string(),
        backend: "bullmq-redis".into(),
        endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
            host: "h.example.com".to_string(),
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
async fn create_appends_in_order_and_lists_empty_groups() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();

    let a = create_group_inner("Lumen", &repo(db.pool())).await.unwrap();
    let b = create_group_inner("  Acme  ", &repo(db.pool()))
        .await
        .unwrap();

    assert_eq!(a.display_order, 0);
    assert_eq!(b.display_order, 1);
    assert_eq!(b.name, "Acme", "name is trimmed");
    assert!(!a.collapsed, "new groups start expanded");
    assert_eq!(a.connection_count, 0);

    let groups = list_groups_inner(&repo(db.pool())).await.unwrap();
    assert_eq!(
        groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
        vec!["Lumen", "Acme"],
        "empty groups are listed, in display order"
    );
}

#[tokio::test]
async fn create_rejects_empty_too_long_and_duplicate_names() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    create_group_inner("Lumen", &repo(db.pool())).await.unwrap();

    for bad in ["", "   "] {
        match create_group_inner(bad, &repo(db.pool())).await {
            Err(RanchError::InvalidInput { reason }) => assert!(reason.contains("empty")),
            other => panic!("expected InvalidInput for {bad:?}, got {other:?}"),
        }
    }

    let too_long = "x".repeat(GROUP_NAME_MAX_CHARS + 1);
    match create_group_inner(&too_long, &repo(db.pool())).await {
        Err(RanchError::InvalidInput { reason }) => assert!(reason.contains("exceed")),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    let at_limit = "x".repeat(GROUP_NAME_MAX_CHARS);
    create_group_inner(&at_limit, &repo(db.pool()))
        .await
        .unwrap();

    match create_group_inner("lUMEN", &repo(db.pool())).await {
        Err(RanchError::InvalidInput { reason }) => assert!(reason.contains("already exists")),
        other => panic!("expected InvalidInput (duplicate), got {other:?}"),
    }
}

#[tokio::test]
async fn rename_validates_and_allows_case_only_change() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let a = create_group_inner("Lumen", &repo(db.pool())).await.unwrap();
    create_group_inner("Acme", &repo(db.pool())).await.unwrap();

    let renamed = rename_group_inner(&a.id, "lumen", &repo(db.pool()))
        .await
        .unwrap();
    assert_eq!(renamed.name, "lumen", "case-only rename of self is fine");

    match rename_group_inner(&a.id, "ACME", &repo(db.pool())).await {
        Err(RanchError::InvalidInput { reason }) => assert!(reason.contains("already exists")),
        other => panic!("expected InvalidInput (duplicate), got {other:?}"),
    }

    match rename_group_inner("nope", "Whatever", &repo(db.pool())).await {
        Err(RanchError::GroupNotFound { id }) => assert_eq!(id, "nope"),
        other => panic!("expected GroupNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn collapsed_state_persists() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let a = create_group_inner("Lumen", &repo(db.pool())).await.unwrap();

    set_group_collapsed_inner(&a.id, true, &repo(db.pool()))
        .await
        .unwrap();
    let groups = list_groups_inner(&repo(db.pool())).await.unwrap();
    assert!(groups[0].collapsed);

    set_group_collapsed_inner(&a.id, false, &repo(db.pool()))
        .await
        .unwrap();
    let groups = list_groups_inner(&repo(db.pool())).await.unwrap();
    assert!(!groups[0].collapsed);

    match set_group_collapsed_inner("nope", true, &repo(db.pool())).await {
        Err(RanchError::GroupNotFound { .. }) => {}
        other => panic!("expected GroupNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn delete_empty_group_needs_no_cascade() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc = keychain();
    let a = create_group_inner("Lumen", &repo(db.pool())).await.unwrap();

    delete_group_inner(&a.id, false, &repo(db.pool()), &kc, None)
        .await
        .unwrap();
    assert!(list_groups_inner(&repo(db.pool()))
        .await
        .unwrap()
        .is_empty());

    match delete_group_inner(&a.id, false, &repo(db.pool()), &kc, None).await {
        Err(RanchError::GroupNotFound { .. }) => {}
        other => panic!("expected GroupNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn delete_non_empty_group_requires_cascade_and_cleans_keychain() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc = keychain();

    let c1 = add_connection_inner(
        add_args("c1", Some("s3cret")),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let c2 = add_connection_inner(
        add_args("c2", Some("other")),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    assert_eq!(c1.group_id, c2.group_id);

    let groups = list_groups_inner(&repo(db.pool())).await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].connection_count, 2);

    // Without cascade: rejected, nothing touched.
    match delete_group_inner(&c1.group_id, false, &repo(db.pool()), &kc, None).await {
        Err(RanchError::GroupNotEmpty {
            connection_count, ..
        }) => assert_eq!(connection_count, 2),
        other => panic!("expected GroupNotEmpty, got {other:?}"),
    }
    assert_eq!(
        list_connections_inner(&repo(db.pool()))
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(kc.get(&c1.id).unwrap().as_deref(), Some("s3cret"));

    // With cascade: group, environments, connections and credentials gone.
    delete_group_inner(&c1.group_id, true, &repo(db.pool()), &kc, None)
        .await
        .unwrap();
    assert!(list_groups_inner(&repo(db.pool()))
        .await
        .unwrap()
        .is_empty());
    assert!(list_connections_inner(&repo(db.pool()))
        .await
        .unwrap()
        .is_empty());
    let (env_count,): (i64,) = sqlx::query_as("SELECT count(*) FROM environments")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(env_count, 0, "environments cascade with the group");
    assert_eq!(kc.get(&c1.id).unwrap(), None);
    assert_eq!(kc.get(&c2.id).unwrap(), None);
}
