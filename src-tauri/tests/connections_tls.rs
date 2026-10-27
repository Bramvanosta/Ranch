//! Integration tests for TLS connection support.
//!
//! Three concerns covered:
//!   1. PEM validation (the `validate_ca_pem` helper, exercised through the
//!      command surface — `add_connection_inner` rejects malformed input
//!      before SQL).
//!   2. Persistence round-trip: add with CA → list returns it → update to
//!      clear → list returns None → duplicate carries the CA forward.
//!   3. Smoke: the `build_redis_client` factory accepts a valid CA without
//!      error (no real network needed — verifies rustls-pemfile parses cleanly).
//!
//! No live TLS handshake here — that requires a running Redis server with
//! matching cert. Manual smoke against Upstash + a private-CA endpoint
//! covers the live path.

use ranch_lib::commands::connections::{
    add_connection_inner, duplicate_connection_inner, fetch_connection_by_id_inner,
    list_connections_inner, update_connection_inner, AddConnectionArgs, UpdateConnectionArgs,
};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::core::db::Db;
use ranch_lib::core::errors::RanchError;
use ranch_lib::core::keychain::InMemoryKeychain;
use ranch_lib::core::tls::{build_redis_client, validate_ca_pem};
use tempfile::tempdir;

const TEST_CA_PEM: &str = include_str!("fixtures/self_signed_ca.pem");

fn add_args(name: &str, tls: bool, tls_custom_ca: Option<&str>) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.to_string(),
        host: "h.example.com".to_string(),
        port: 6379,
        db: 0,
        username: None,
        password: None,
        tls,
        tls_custom_ca: tls_custom_ca.map(str::to_string),
        ssh_tunnel: None,
        ssh_password: None,
        group: GroupSelector::named("Default"),
        environment: EnvironmentSelector::for_color("env-local"),
    }
}

// ── 1. PEM validation through the command surface ─────────────────────────

#[tokio::test]
async fn add_connection_rejects_garbage_pem() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let result =
        add_connection_inner(add_args("c1", true, Some("not a pem")), db.pool(), &kc).await;

    match result {
        Err(RanchError::InvalidInput { reason }) => {
            assert!(reason.contains("Invalid CA PEM"), "got: {reason}");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }

    // Defensive — verify no row was created (the validation runs before tx).
    let connections = list_connections_inner(db.pool()).await.unwrap();
    assert!(
        connections.is_empty(),
        "row should not be created on bad PEM"
    );
}

#[tokio::test]
async fn add_connection_rejects_private_key_pem() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let pkcs8 = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEINTuctv5E1hK1bbY8fdp+K06/y/c6w==\n-----END PRIVATE KEY-----\n";
    let result = add_connection_inner(add_args("c1", true, Some(pkcs8)), db.pool(), &kc).await;
    assert!(matches!(result, Err(RanchError::InvalidInput { .. })));
}

#[tokio::test]
async fn add_connection_normalizes_empty_pem_to_none() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    // Some("") and Some("   ") both normalize to None — neither errors,
    // neither persists a value.
    let conn = add_connection_inner(add_args("empty", true, Some("")), db.pool(), &kc)
        .await
        .unwrap();
    assert_eq!(conn.tls_custom_ca, None);

    let conn2 = add_connection_inner(add_args("ws", true, Some("   \n  \t  ")), db.pool(), &kc)
        .await
        .unwrap();
    assert_eq!(conn2.tls_custom_ca, None);
}

// ── 2. Persistence round-trip ─────────────────────────────────────────────

#[tokio::test]
async fn add_with_valid_ca_persists_and_lists() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(add_args("with-ca", true, Some(TEST_CA_PEM)), db.pool(), &kc)
        .await
        .unwrap();
    assert!(conn.tls);
    assert!(conn.tls_custom_ca.is_some());
    let pem_in = conn.tls_custom_ca.unwrap();
    assert!(pem_in.contains("-----BEGIN CERTIFICATE-----"));

    // list_connections returns the same value
    let listed = list_connections_inner(db.pool()).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].tls_custom_ca.as_deref(), Some(pem_in.as_str()));

    // fetch_connection_by_id returns the same
    let fetched = fetch_connection_by_id_inner(&listed[0].id, db.pool())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.tls_custom_ca, Some(pem_in));
}

#[tokio::test]
async fn update_clears_ca_when_passed_empty_string() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(add_args("c1", true, Some(TEST_CA_PEM)), db.pool(), &kc)
        .await
        .unwrap();
    assert!(conn.tls_custom_ca.is_some());

    // Some("") explicitly clears the column (unlike password).
    let updated = update_connection_inner(
        UpdateConnectionArgs {
            locked: None,
            id: conn.id.clone(),
            name: conn.name.clone(),
            host: conn.host.clone(),
            port: conn.port,
            db: conn.db,
            username: conn.username.clone(),
            password: None,
            tls: conn.tls,
            tls_custom_ca: Some("".to_string()),
            ssh_tunnel: None,
            ssh_tunnel_clear: false,
            ssh_password: None,
            group: GroupSelector::Existing {
                id: conn.group_id.clone(),
            },
            environment: EnvironmentSelector::Existing {
                id: conn.environment_id.clone(),
            },
        },
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    assert_eq!(updated.tls_custom_ca, None);

    // Round-trip via list to confirm.
    let listed = list_connections_inner(db.pool()).await.unwrap();
    assert_eq!(listed[0].tls_custom_ca, None);
}

#[tokio::test]
async fn update_with_none_leaves_ca_untouched() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(add_args("c1", true, Some(TEST_CA_PEM)), db.pool(), &kc)
        .await
        .unwrap();

    // Update with tls_custom_ca: None → column untouched.
    let updated = update_connection_inner(
        UpdateConnectionArgs {
            locked: None,
            id: conn.id.clone(),
            name: "renamed".to_string(),
            host: conn.host.clone(),
            port: conn.port,
            db: conn.db,
            username: conn.username.clone(),
            password: None,
            tls: conn.tls,
            tls_custom_ca: None, // ← leave alone
            ssh_tunnel: None,
            ssh_tunnel_clear: false,
            ssh_password: None,
            group: GroupSelector::Existing {
                id: conn.group_id.clone(),
            },
            environment: EnvironmentSelector::Existing {
                id: conn.environment_id.clone(),
            },
        },
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    assert_eq!(updated.name, "renamed");
    assert!(
        updated.tls_custom_ca.is_some(),
        "CA should still be present"
    );
}

#[tokio::test]
async fn update_rejects_garbage_pem() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(add_args("c1", true, None), db.pool(), &kc)
        .await
        .unwrap();

    let err = update_connection_inner(
        UpdateConnectionArgs {
            locked: None,
            id: conn.id.clone(),
            name: conn.name.clone(),
            host: conn.host.clone(),
            port: conn.port,
            db: conn.db,
            username: conn.username.clone(),
            password: None,
            tls: conn.tls,
            tls_custom_ca: Some("not a pem".to_string()),
            ssh_tunnel: None,
            ssh_tunnel_clear: false,
            ssh_password: None,
            group: GroupSelector::Existing {
                id: conn.group_id.clone(),
            },
            environment: EnvironmentSelector::Existing {
                id: conn.environment_id.clone(),
            },
        },
        db.pool(),
        &kc,
    )
    .await;
    assert!(matches!(err, Err(RanchError::InvalidInput { .. })));
}

#[tokio::test]
async fn duplicate_carries_ca_forward() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(add_args("orig", true, Some(TEST_CA_PEM)), db.pool(), &kc)
        .await
        .unwrap();

    let dup = duplicate_connection_inner(&conn.id, db.pool(), &kc)
        .await
        .unwrap();
    assert_eq!(dup.tls_custom_ca, conn.tls_custom_ca);
    assert!(dup.tls);
    // The duplicate is its own row.
    assert_ne!(dup.id, conn.id);
}

// ── 3. Build factory smoke ────────────────────────────────────────────────

#[test]
fn build_factory_accepts_valid_ca() {
    let client = build_redis_client("rediss://example.invalid:6379/0", Some(TEST_CA_PEM));
    assert!(
        client.is_ok(),
        "build_redis_client should accept the test CA: {:?}",
        client.err()
    );
}

#[test]
fn build_factory_no_ca_falls_back_to_open() {
    let client = build_redis_client("redis://127.0.0.1:6379/0", None);
    assert!(client.is_ok());
}

#[test]
fn validate_ca_pem_accepts_test_fixture() {
    let result = validate_ca_pem(Some(TEST_CA_PEM)).unwrap();
    assert!(result.is_some());
}
