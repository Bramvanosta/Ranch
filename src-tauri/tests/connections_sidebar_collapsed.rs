//! A connection's folded state in the sidebar persists
//! (migration 0008, `set_connection_collapsed`).

mod common;
use common::repo;

use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{
    add_connection_inner, list_connections_inner, set_connection_collapsed_inner, AddConnectionArgs,
};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::domain::errors::RanchError;
use std::sync::Arc;
use tempfile::tempdir;

fn args(name: &str) -> AddConnectionArgs {
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
        password: None,
        ssh_password: None,
        group: GroupSelector::named("Lumen"),
        environment: EnvironmentSelector::for_color("env-local"),
    }
}

#[tokio::test]
async fn a_new_connection_is_unfolded_and_folding_persists() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let added = add_connection_inner(
        args("local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    assert!(!added.sidebar_collapsed);

    set_connection_collapsed_inner(&added.id, true, &repo(db.pool()))
        .await
        .unwrap();
    let listed = list_connections_inner(&repo(db.pool())).await.unwrap();
    assert!(listed[0].sidebar_collapsed);

    set_connection_collapsed_inner(&added.id, false, &repo(db.pool()))
        .await
        .unwrap();
    let listed = list_connections_inner(&repo(db.pool())).await.unwrap();
    assert!(!listed[0].sidebar_collapsed);
}

#[tokio::test]
async fn folding_an_unknown_connection_is_an_error() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    match set_connection_collapsed_inner("nope", true, &repo(db.pool())).await {
        Err(RanchError::ConnectionNotFound { id }) => assert_eq!(id, "nope"),
        other => panic!("expected ConnectionNotFound, got {other:?}"),
    }
}
