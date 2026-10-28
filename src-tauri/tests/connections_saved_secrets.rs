//! A saved password goes back only to the server it was saved for: editing a
//! connection's host (or turning TLS off) with "leave blank to keep" asks for
//! the password again instead of handing the old one to the new host.

mod common;
use common::repo;

use std::sync::Arc;

use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{
    add_connection_inner, update_connection_inner, AddConnectionArgs, BullmqRedisEndpoint,
    Connection, UpdateConnectionArgs,
};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::domain::errors::RanchError;
use tempfile::tempdir;

fn add(password: Option<&str>) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: "api".to_string(),
        backend: "bullmq-redis".into(),
        endpoint: BullmqRedisEndpoint {
            host: "redis.internal".to_string(),
            port: 6380,
            db: 0,
            username: None,
            tls: true,
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

/// An edit of the saved connection, its endpoint changed by `change`.
fn edit_with(
    conn: &Connection,
    change: impl FnOnce(&mut BullmqRedisEndpoint),
) -> UpdateConnectionArgs {
    let mut endpoint = BullmqRedisEndpoint::read(&conn.endpoint).unwrap();
    change(&mut endpoint);
    UpdateConnectionArgs {
        endpoint: endpoint.json(),
        ..edit(conn)
    }
}

fn edit(conn: &Connection) -> UpdateConnectionArgs {
    UpdateConnectionArgs {
        locked: None,
        id: conn.id.clone(),
        name: conn.name.clone(),
        backend: conn.backend.clone(),
        endpoint: conn.endpoint.clone(),
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

#[tokio::test]
async fn a_kept_password_is_refused_for_another_endpoint() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        add(Some("s3cret")),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    for moved in [
        edit_with(&conn, |e| e.host = "attacker.example".into()),
        edit_with(&conn, |e| e.port = 6379),
        edit_with(&conn, |e| e.tls = false),
    ] {
        let err = update_connection_inner(
            moved,
            &repo(db.pool()),
            &kc,
            &ranch_lib::adapters::bullmq_redis::specs(),
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, RanchError::InvalidInput { .. }), "{err:?}");
    }

    // Same endpoint (another name, host in other case): the password stays.
    let renamed = UpdateConnectionArgs {
        name: "api 2".into(),
        ..edit_with(&conn, |e| e.host = "REDIS.internal".into())
    };
    update_connection_inner(
        renamed,
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(kc.get(&conn.id).unwrap().as_deref(), Some("s3cret"));

    // A new host with the password typed again is fine.
    let retyped = UpdateConnectionArgs {
        password: Some("n3w".into()),
        ..edit_with(&conn, |e| e.host = "redis-2.internal".into())
    };
    update_connection_inner(
        retyped,
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(kc.get(&conn.id).unwrap().as_deref(), Some("n3w"));
}

#[tokio::test]
async fn a_connection_without_a_password_moves_freely() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        add(None),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let moved = edit_with(&conn, |e| e.host = "elsewhere.internal".into());
    update_connection_inner(
        moved,
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
        None,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn control_characters_in_credentials_are_refused() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let err = add_connection_inner(
        add(Some("x\r\nCONFIG SET dir /tmp")),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, RanchError::InvalidInput { .. }), "{err:?}");
}
