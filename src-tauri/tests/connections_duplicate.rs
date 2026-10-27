//! Integration tests for `duplicate_connection_inner`.

use ranch_lib::commands::connections::{
    add_connection_inner, duplicate_connection_inner, list_connections_inner, AddConnectionArgs,
};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::core::db::Db;
use ranch_lib::core::errors::RanchError;
use ranch_lib::core::keychain::InMemoryKeychain;
use tempfile::tempdir;

fn args(name: &str, password: Option<&str>) -> AddConnectionArgs {
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
        environment: EnvironmentSelector::for_color("env-local"),
    }
}

#[tokio::test]
async fn duplicate_creates_copy_with_independent_credential() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let original = add_connection_inner(args("foo", Some("secret")), db.pool(), &kc)
        .await
        .unwrap();
    let copy = duplicate_connection_inner(&original.id, db.pool(), &kc)
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
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let original = add_connection_inner(args("foo", None), db.pool(), &kc)
        .await
        .unwrap();
    let first_copy = duplicate_connection_inner(&original.id, db.pool(), &kc)
        .await
        .unwrap();
    assert_eq!(first_copy.name, "foo (copy)");

    // Duplicating the duplicate strips its " (copy)" suffix and counts again.
    let second_copy = duplicate_connection_inner(&first_copy.id, db.pool(), &kc)
        .await
        .unwrap();
    assert_eq!(second_copy.name, "foo (copy 2)");

    let listed = list_connections_inner(db.pool()).await.unwrap();
    let names: Vec<&str> = listed.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"foo"));
    assert!(names.contains(&"foo (copy)"));
    assert!(names.contains(&"foo (copy 2)"));
}

#[tokio::test]
async fn duplicate_auth_less_connection_skips_keychain() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let original = add_connection_inner(args("foo", None), db.pool(), &kc)
        .await
        .unwrap();
    let copy = duplicate_connection_inner(&original.id, db.pool(), &kc)
        .await
        .unwrap();

    // Neither has a credential.
    assert_eq!(kc.get(&original.id).unwrap(), None);
    assert_eq!(kc.get(&copy.id).unwrap(), None);
}

#[tokio::test]
async fn duplicate_unknown_id_returns_connection_not_found() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let err = duplicate_connection_inner("nope", db.pool(), &kc)
        .await
        .unwrap_err();
    assert!(matches!(err, RanchError::ConnectionNotFound { .. }));
}
