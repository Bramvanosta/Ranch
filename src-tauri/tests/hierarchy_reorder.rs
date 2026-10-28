//! Integration tests for `reorder_groups_inner`.

mod common;
use common::repo;

use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::reorder_groups_inner;
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
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
