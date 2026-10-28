//! Integration tests for TLS connection support.
//!
//! Three concerns covered:
//!   1. PEM validation (the `validate_ca_pem` helper, exercised through the
//!      command surface: `add_connection_inner` rejects malformed input
//!      before SQL).
//!   2. Persistence round-trip: add with CA → list returns it → update to
//!      clear → list returns None → duplicate carries the CA forward. The CA
//!      lives in the connection's endpoint.
//!   3. Smoke: the `build_redis_client` factory accepts a valid CA without
//!      error (no real network needed; verifies rustls-pemfile parses cleanly).
//!
//! No live TLS handshake here: that requires a running Redis server with
//! matching cert. Manual smoke against a managed Redis + a private-CA endpoint
//! covers the live path.

mod common;
use common::repo;

use ranch_lib::adapters::bullmq_redis::tls::{build_redis_client, validate_ca_pem};
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::commands::connections::{
    add_connection_inner, duplicate_connection_inner, fetch_connection_by_id_inner,
    list_connections_inner, update_connection_inner, AddConnectionArgs, BullmqRedisEndpoint,
    Connection, UpdateConnectionArgs,
};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::domain::errors::RanchError;
use tempfile::tempdir;

const TEST_CA_PEM: &str = include_str!("fixtures/self_signed_ca.pem");

/// The connection's Redis endpoint, as the form reads it back.
fn ep(conn: &Connection) -> BullmqRedisEndpoint {
    BullmqRedisEndpoint::read(&conn.endpoint).expect("a Redis endpoint")
}

/// An edit that sends the saved endpoint back, changed by `change`.
fn edit_with(
    conn: &Connection,
    change: impl FnOnce(&mut BullmqRedisEndpoint),
) -> UpdateConnectionArgs {
    let mut endpoint = ep(conn);
    change(&mut endpoint);
    UpdateConnectionArgs {
        locked: None,
        id: conn.id.clone(),
        name: conn.name.clone(),
        backend: conn.backend.clone(),
        endpoint: endpoint.json(),
        password: None,
        ssh_password: None,
        group: GroupSelector::Existing {
            id: conn.group_id.clone(),
        },
        environment: EnvironmentSelector::Existing {
            id: conn.environment_id.clone(),
        },
    }
}

fn add_args(name: &str, tls: bool, tls_custom_ca: Option<&str>) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.to_string(),
        backend: "bullmq-redis".into(),
        endpoint: BullmqRedisEndpoint {
            host: "h.example.com".to_string(),
            port: 6379,
            db: 0,
            username: None,
            tls,
            tls_custom_ca: tls_custom_ca.map(str::to_string),
            ssh_tunnel: None,
        }
        .json(),
        password: None,
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
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let result = add_connection_inner(
        add_args("c1", true, Some("not a pem")),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await;

    match result {
        Err(RanchError::InvalidInput { reason }) => {
            assert!(reason.contains("Invalid CA PEM"), "got: {reason}");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }

    // Defensive: verify no row was created (the validation runs before tx).
    let connections = list_connections_inner(&repo(db.pool())).await.unwrap();
    assert!(
        connections.is_empty(),
        "row should not be created on bad PEM"
    );
}

#[tokio::test]
async fn add_connection_rejects_private_key_pem() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let pkcs8 = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEINTuctv5E1hK1bbY8fdp+K06/y/c6w==\n-----END PRIVATE KEY-----\n";
    let result = add_connection_inner(
        add_args("c1", true, Some(pkcs8)),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await;
    assert!(matches!(result, Err(RanchError::InvalidInput { .. })));
}

#[tokio::test]
async fn add_connection_normalizes_empty_pem_to_none() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    // Some("") and Some("   ") both normalize to None; neither errors,
    // neither persists a value.
    let conn = add_connection_inner(
        add_args("empty", true, Some("")),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    assert_eq!(ep(&conn).tls_custom_ca, None);

    let conn2 = add_connection_inner(
        add_args("ws", true, Some("   \n  \t  ")),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    assert_eq!(ep(&conn2).tls_custom_ca, None);
}

// ── 2. Persistence round-trip ─────────────────────────────────────────────

#[tokio::test]
async fn add_with_valid_ca_persists_and_lists() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        add_args("with-ca", true, Some(TEST_CA_PEM)),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    assert!(ep(&conn).tls);
    assert!(ep(&conn).tls_custom_ca.is_some());
    let pem_in = ep(&conn).tls_custom_ca.unwrap();
    assert!(pem_in.contains("-----BEGIN CERTIFICATE-----"));

    // list_connections returns the same value
    let listed = list_connections_inner(&repo(db.pool())).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        ep(&listed[0]).tls_custom_ca.as_deref(),
        Some(pem_in.as_str())
    );

    // fetch_connection_by_id returns the same
    let fetched = fetch_connection_by_id_inner(&listed[0].id, &repo(db.pool()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ep(&fetched).tls_custom_ca, Some(pem_in));
}

#[tokio::test]
async fn update_clears_ca_when_passed_empty_string() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        add_args("c1", true, Some(TEST_CA_PEM)),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    assert!(ep(&conn).tls_custom_ca.is_some());

    // An empty CA is no CA (unlike password).
    let updated = update_connection_inner(
        edit_with(&conn, |e| e.tls_custom_ca = Some("".to_string())),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(ep(&updated).tls_custom_ca, None);

    // Round-trip via list to confirm.
    let listed = list_connections_inner(&repo(db.pool())).await.unwrap();
    assert_eq!(ep(&listed[0]).tls_custom_ca, None);
}

#[tokio::test]
async fn update_sending_the_endpoint_back_keeps_the_ca() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        add_args("c1", true, Some(TEST_CA_PEM)),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    // The form sends the whole endpoint back, CA included.
    let updated = update_connection_inner(
        UpdateConnectionArgs {
            name: "renamed".to_string(),
            ..edit_with(&conn, |_| {})
        },
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(updated.name, "renamed");
    assert!(
        ep(&updated).tls_custom_ca.is_some(),
        "CA should still be present"
    );
}

#[tokio::test]
async fn update_rejects_garbage_pem() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        add_args("c1", true, None),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    let err = update_connection_inner(
        edit_with(&conn, |e| e.tls_custom_ca = Some("not a pem".to_string())),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
        None,
    )
    .await;
    assert!(matches!(err, Err(RanchError::InvalidInput { .. })));
}

#[tokio::test]
async fn duplicate_carries_ca_forward() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        add_args("orig", true, Some(TEST_CA_PEM)),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    let dup = duplicate_connection_inner(&conn.id, &repo(db.pool()), &kc)
        .await
        .unwrap();
    assert_eq!(ep(&dup).tls_custom_ca, ep(&conn).tls_custom_ca);
    assert!(ep(&dup).tls);
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
