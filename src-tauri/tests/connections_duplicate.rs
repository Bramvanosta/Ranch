//! Integration tests for `duplicate_connection_inner`.

mod common;
use common::repo;

use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::commands::connections::{
    add_connection_inner, duplicate_connection_inner, list_connections_inner,
    ssh_hostkey_keychain_name, ssh_jump_hostkey_keychain_name, AddConnectionArgs,
};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::domain::errors::RanchError;
use tempfile::tempdir;

fn args(name: &str, password: Option<&str>) -> AddConnectionArgs {
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
        password: password.map(str::to_string),
        ssh_password: None,
        group: GroupSelector::named("Default"),
        environment: EnvironmentSelector::for_color("env-local"),
    }
}

#[tokio::test]
async fn duplicate_creates_copy_with_independent_credential() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let original = add_connection_inner(
        args("foo", Some("secret")),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let copy = duplicate_connection_inner(&original.id, &repo(db.pool()), &kc)
        .await
        .unwrap();

    assert_ne!(copy.id, original.id);
    assert_eq!(copy.name, "foo (copy)");
    assert_eq!(copy.environment_id, original.environment_id);

    // Independent credential: both keychain entries hold the same secret.
    assert_eq!(kc.get(&original.id).unwrap().as_deref(), Some("secret"));
    assert_eq!(kc.get(&copy.id).unwrap().as_deref(), Some("secret"));

    // Modifying source keychain does not affect copy keychain.
    kc.set(&original.id, "changed").unwrap();
    assert_eq!(kc.get(&copy.id).unwrap().as_deref(), Some("secret"));
}

#[tokio::test]
async fn duplicate_of_duplicate_increments_suffix() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let original = add_connection_inner(
        args("foo", None),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let first_copy = duplicate_connection_inner(&original.id, &repo(db.pool()), &kc)
        .await
        .unwrap();
    assert_eq!(first_copy.name, "foo (copy)");

    // Duplicating the duplicate strips its " (copy)" suffix and counts again.
    let second_copy = duplicate_connection_inner(&first_copy.id, &repo(db.pool()), &kc)
        .await
        .unwrap();
    assert_eq!(second_copy.name, "foo (copy 2)");

    let listed = list_connections_inner(&repo(db.pool())).await.unwrap();
    let names: Vec<&str> = listed.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"foo"));
    assert!(names.contains(&"foo (copy)"));
    assert!(names.contains(&"foo (copy 2)"));
}

#[tokio::test]
async fn duplicate_auth_less_connection_skips_keychain() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let original = add_connection_inner(
        args("foo", None),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let copy = duplicate_connection_inner(&original.id, &repo(db.pool()), &kc)
        .await
        .unwrap();

    // Neither has a credential.
    assert_eq!(kc.get(&original.id).unwrap(), None);
    assert_eq!(kc.get(&copy.id).unwrap(), None);
}

#[tokio::test]
async fn duplicate_keeps_the_pinned_host_keys() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let original = add_connection_inner(
        args("foo", None),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    kc.set(&ssh_hostkey_keychain_name(&original.id), "SHA256:bastion")
        .unwrap();
    kc.set(&ssh_jump_hostkey_keychain_name(&original.id), "SHA256:jump")
        .unwrap();
    let copy = duplicate_connection_inner(&original.id, &repo(db.pool()), &kc)
        .await
        .unwrap();

    // The copy reaches the same bastion: it does not trust whatever answers
    // its first connect.
    assert_eq!(
        kc.get(&ssh_hostkey_keychain_name(&copy.id))
            .unwrap()
            .as_deref(),
        Some("SHA256:bastion")
    );
    assert_eq!(
        kc.get(&ssh_jump_hostkey_keychain_name(&copy.id))
            .unwrap()
            .as_deref(),
        Some("SHA256:jump")
    );
}

#[tokio::test]
async fn duplicate_unknown_id_returns_connection_not_found() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let err = duplicate_connection_inner("nope", &repo(db.pool()), &kc)
        .await
        .unwrap_err();
    assert!(matches!(err, RanchError::ConnectionNotFound { .. }));
}

#[tokio::test]
async fn duplicate_of_a_production_connection_keeps_its_protections() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let prod = AddConnectionArgs {
        environment: EnvironmentSelector::for_color("env-production"),
        ..args("prod", Some("secret"))
    };
    let original = add_connection_inner(
        prod,
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let copy = duplicate_connection_inner(&original.id, &repo(db.pool()), &kc)
        .await
        .unwrap();

    // Same server, same credentials: as locked and as read-only for agents.
    let row = |id: String| {
        let pool = db.pool().clone();
        async move {
            sqlx::query_as::<_, (bool, Option<bool>)>(
                "SELECT is_locked, mcp_readonly FROM connections WHERE id = ?",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let source = row(original.id.clone()).await;
    assert_eq!(source, (true, Some(true)), "production starts protected");
    assert_eq!(row(copy.id.clone()).await, source);
    assert_eq!(copy.mcp_readonly, Some(true));
}
