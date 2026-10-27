//! Integration test for `list_connections_inner`.
//!
//! Verifies:
//!   - Empty database returns an empty list.
//!   - Connections are returned in deterministic order (by group display_order,
//!     then env display_order, then connection created_at).
//!   - Joined env / group fields land correctly in the returned struct.
//!
//! Hermetic: tempdir SQLite + InMemoryKeychain.

use ranch_lib::commands::connections::{
    add_connection_inner, list_connections_inner, AddConnectionArgs,
};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::core::db::Db;
use ranch_lib::core::keychain::InMemoryKeychain;
use tempfile::tempdir;

fn args(name: &str, color: &str) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.to_string(),
        host: "redis.example.com".to_string(),
        port: 6379,
        db: 0,
        username: None,
        password: None,
        tls: false,
        tls_custom_ca: None,
        ssh_tunnel: None,
        ssh_password: None,
        group: GroupSelector::named("Default"),
        environment: EnvironmentSelector::for_color(color),
    }
}

#[tokio::test]
async fn list_is_empty_on_fresh_db() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");

    let connections = list_connections_inner(db.pool()).await.expect("list");
    assert!(connections.is_empty());
}

#[tokio::test]
async fn list_returns_inserted_connections_with_joined_fields() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let inserted = [
        add_connection_inner(args("alpha", "env-local"), db.pool(), &kc)
            .await
            .expect("alpha"),
        add_connection_inner(args("beta", "env-staging"), db.pool(), &kc)
            .await
            .expect("beta"),
        add_connection_inner(args("gamma", "env-production"), db.pool(), &kc)
            .await
            .expect("gamma"),
    ];

    let listed = list_connections_inner(db.pool()).await.expect("list");
    assert_eq!(listed.len(), 3);

    // Each inserted id must be present in the listing.
    for c in &inserted {
        let found = listed.iter().find(|x| x.id == c.id).expect("missing id");
        assert_eq!(found.name, c.name);
        assert_eq!(found.env_color, c.env_color);
        assert_eq!(found.group_name, "Default");
        // Joined fields must reflect the env row created during add.
        // The default env label mirrors the
        // color-derived name (e.g. "Local") rather than the generic
        // "Environment" label used pre-fix.
        assert!(!found.env_label.is_empty());
        assert_eq!(found.env_label, found.env_name);
        assert!(!found.env_name.is_empty());
    }
}

#[tokio::test]
async fn list_preserves_insertion_order_within_same_environment() {
    // Same color → same env → ORDER BY created_at means the insertion
    // sequence is preserved.
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("db init");
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let names = ["one", "two", "three"];
    for name in names {
        add_connection_inner(args(name, "env-local"), db.pool(), &kc)
            .await
            .expect("insert");
        // Sleep tiny bit so the rfc3339 created_at strings sort properly.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let listed = list_connections_inner(db.pool()).await.expect("list");
    let order: Vec<&str> = listed.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(order, vec!["one", "two", "three"]);
}
