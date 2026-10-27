//! Integration tests for explicit group / environment placement
//! and `change_environment_color` (the color is a plain attribute of the
//! environment; changing it never moves a connection).

use ranch_lib::commands::connections::{
    add_connection_inner, list_connections_inner, update_connection_inner, AddConnectionArgs,
    Connection, UpdateConnectionArgs,
};
use ranch_lib::commands::groups::{create_group_inner, list_groups_inner};
use ranch_lib::commands::hierarchy::{
    change_environment_color_inner, default_env_name_for_color, EnvironmentSelector, GroupSelector,
};
use ranch_lib::core::db::Db;
use ranch_lib::core::errors::RanchError;
use ranch_lib::core::keychain::{InMemoryKeychain, KeychainAdapter};
use std::sync::Arc;
use tempfile::tempdir;

fn keychain() -> Arc<dyn KeychainAdapter> {
    Arc::new(InMemoryKeychain::new())
}

fn add_args(
    name: &str,
    group: GroupSelector,
    environment: EnvironmentSelector,
) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.to_string(),
        host: "h.example.com".to_string(),
        port: 6379,
        db: 0,
        username: None,
        password: None,
        tls: false,
        tls_custom_ca: None,
        ssh_tunnel: None,
        ssh_password: None,
        group,
        environment,
    }
}

fn update_args(
    conn: &Connection,
    group: GroupSelector,
    environment: EnvironmentSelector,
) -> UpdateConnectionArgs {
    UpdateConnectionArgs {
        locked: None,
        id: conn.id.clone(),
        name: conn.name.clone(),
        host: conn.host.clone(),
        port: conn.port,
        db: conn.db,
        username: None,
        password: None,
        tls: false,
        tls_custom_ca: None,
        ssh_tunnel: None,
        ssh_tunnel_clear: false,
        ssh_password: None,
        group,
        environment,
    }
}

async fn is_locked(db: &Db, id: &str) -> bool {
    let (locked,): (i64,) = sqlx::query_as("SELECT is_locked FROM connections WHERE id = ?")
        .bind(id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    locked != 0
}

#[tokio::test]
async fn one_group_per_project_one_environment_per_stage() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc = keychain();

    let local = add_connection_inner(
        add_args(
            "lumen-local",
            GroupSelector::named("Lumen"),
            EnvironmentSelector::named("Local", "env-local"),
        ),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    // Same group by name (different casing) → reused, not duplicated.
    let prod = add_connection_inner(
        add_args(
            "lumen-prod",
            GroupSelector::named("lumen"),
            EnvironmentSelector::named("Production", "env-production"),
        ),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();

    assert_eq!(local.group_id, prod.group_id);
    assert_eq!(local.group_name, "Lumen");
    assert_ne!(local.environment_id, prod.environment_id);
    assert_eq!(prod.env_label, "Production");
    assert_eq!(prod.env_color, "env-production");

    let groups = list_groups_inner(db.pool()).await.unwrap();
    assert_eq!(groups.len(), 1, "no auto-created \"Default\" group anymore");
    assert_eq!(groups[0].connection_count, 2);

    assert!(!is_locked(&db, &local.id).await);
    assert!(is_locked(&db, &prod.id).await, "production starts locked");
}

#[tokio::test]
async fn color_never_picks_the_environment() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc = keychain();

    // Two environments sharing a color stay two environments.
    let a = add_connection_inner(
        add_args(
            "a",
            GroupSelector::named("Acme"),
            EnvironmentSelector::named("EU", "env-staging"),
        ),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    let b = add_connection_inner(
        add_args(
            "b",
            GroupSelector::named("Acme"),
            EnvironmentSelector::named("US", "env-staging"),
        ),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    assert_ne!(a.environment_id, b.environment_id);

    // Same label → the existing environment (and ITS color) wins.
    let c = add_connection_inner(
        add_args(
            "c",
            GroupSelector::named("Acme"),
            EnvironmentSelector::named("eu", "env-custom-4"),
        ),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    assert_eq!(c.environment_id, a.environment_id);
    assert_eq!(c.env_color, "env-staging");
}

#[tokio::test]
async fn existing_selectors_are_validated() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc = keychain();

    let seed = add_connection_inner(
        add_args(
            "seed",
            GroupSelector::named("Acme"),
            EnvironmentSelector::named("Local", "env-local"),
        ),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    let other = create_group_inner("Other", db.pool()).await.unwrap();

    // Unknown group.
    match add_connection_inner(
        add_args(
            "x",
            GroupSelector::Existing { id: "nope".into() },
            EnvironmentSelector::named("Local", "env-local"),
        ),
        db.pool(),
        &kc,
    )
    .await
    {
        Err(RanchError::GroupNotFound { .. }) => {}
        other => panic!("expected GroupNotFound, got {other:?}"),
    }

    // Environment from another group.
    match add_connection_inner(
        add_args(
            "x",
            GroupSelector::Existing {
                id: other.id.clone(),
            },
            EnvironmentSelector::Existing {
                id: seed.environment_id.clone(),
            },
        ),
        db.pool(),
        &kc,
    )
    .await
    {
        Err(RanchError::InvalidInput { reason }) => assert!(reason.contains("does not belong")),
        other => panic!("expected InvalidInput, got {other:?}"),
    }

    // Unknown color, empty names.
    for (group, environment) in [
        (
            GroupSelector::named("Acme"),
            EnvironmentSelector::named("QA", "env-rainbow"),
        ),
        (
            GroupSelector::named("   "),
            EnvironmentSelector::named("QA", "env-local"),
        ),
        (
            GroupSelector::named("Acme"),
            EnvironmentSelector::named("", "env-local"),
        ),
    ] {
        match add_connection_inner(add_args("x", group, environment), db.pool(), &kc).await {
            Err(RanchError::InvalidInput { .. }) => {}
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    // Nothing leaked from the rejected attempts (single transaction).
    assert_eq!(list_connections_inner(db.pool()).await.unwrap().len(), 1);
    assert_eq!(list_groups_inner(db.pool()).await.unwrap().len(), 2);
}

#[tokio::test]
async fn update_moves_the_connection_and_locks_it_when_entering_production() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc = keychain();

    let conn = add_connection_inner(
        add_args(
            "redis",
            GroupSelector::named("Acme"),
            EnvironmentSelector::named("Staging", "env-staging"),
        ),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    assert!(!is_locked(&db, &conn.id).await);

    // Move to a new group + production environment.
    let moved = update_connection_inner(
        update_args(
            &conn,
            GroupSelector::named("Lumen"),
            EnvironmentSelector::named("Production", "env-production"),
        ),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    assert_eq!(moved.id, conn.id, "a move keeps the connection id");
    assert_eq!(moved.group_name, "Lumen");
    assert_eq!(moved.env_color, "env-production");
    assert!(is_locked(&db, &conn.id).await);

    // Moving back out never unlocks.
    let back = update_connection_inner(
        update_args(
            &moved,
            GroupSelector::Existing {
                id: conn.group_id.clone(),
            },
            EnvironmentSelector::Existing {
                id: conn.environment_id.clone(),
            },
        ),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    assert_eq!(back.environment_id, conn.environment_id);
    assert!(is_locked(&db, &conn.id).await);
}

#[tokio::test]
async fn change_environment_color_recolors_in_place_and_locks_on_production() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc = keychain();

    let conn = add_connection_inner(
        add_args(
            "redis",
            GroupSelector::named("Acme"),
            EnvironmentSelector::named("Live", "env-custom-2"),
        ),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();

    let newly_locked =
        change_environment_color_inner(&conn.environment_id, "env-staging", db.pool())
            .await
            .unwrap();
    assert!(newly_locked.is_empty());
    let row = &list_connections_inner(db.pool()).await.unwrap()[0];
    assert_eq!(row.env_color, "env-staging");
    assert_eq!(
        row.environment_id, conn.environment_id,
        "recolored in place, not moved"
    );
    assert_eq!(row.env_label, "Live", "the name does not follow the color");

    let newly_locked =
        change_environment_color_inner(&conn.environment_id, "env-production", db.pool())
            .await
            .unwrap();
    assert_eq!(newly_locked, vec![conn.id.clone()]);
    assert!(is_locked(&db, &conn.id).await);

    // Already locked → nothing new to refresh.
    let again = change_environment_color_inner(&conn.environment_id, "env-production", db.pool())
        .await
        .unwrap();
    assert!(again.is_empty());

    match change_environment_color_inner("nope", "env-local", db.pool()).await {
        Err(RanchError::EnvironmentNotFound { .. }) => {}
        other => panic!("expected EnvironmentNotFound, got {other:?}"),
    }
    match change_environment_color_inner(&conn.environment_id, "env-rainbow", db.pool()).await {
        Err(RanchError::InvalidInput { .. }) => {}
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn palette_default_names() {
    assert_eq!(default_env_name_for_color("env-production"), "Production");
    assert_eq!(default_env_name_for_color("env-staging"), "Staging");
    assert_eq!(default_env_name_for_color("env-local"), "Local");
    assert_eq!(default_env_name_for_color("env-custom-3"), "Custom 3");
}

// ── Environments as entities + moving connections ───────────────

use ranch_lib::commands::connections::list_environments_inner;
use ranch_lib::commands::hierarchy::{
    create_environment_inner, delete_environment_inner, move_connection_inner,
};

#[tokio::test]
async fn create_and_delete_empty_environment() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let group = create_group_inner("Lumen", db.pool()).await.unwrap();

    let env_id = create_environment_inner(&group.id, "  Staging ", "env-staging", db.pool())
        .await
        .unwrap();
    let envs = list_environments_inner(db.pool()).await.unwrap();
    assert_eq!(envs.len(), 1);
    assert_eq!(envs[0].label, "Staging");
    assert_eq!(envs[0].group_name, "Lumen");

    // Explicit create rejects a duplicate label (the form's find-or-create does not).
    match create_environment_inner(&group.id, "staging", "env-local", db.pool()).await {
        Err(RanchError::InvalidInput { reason }) => assert!(reason.contains("already exists")),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    match create_environment_inner("nope", "QA", "env-local", db.pool()).await {
        Err(RanchError::GroupNotFound { .. }) => {}
        other => panic!("expected GroupNotFound, got {other:?}"),
    }

    delete_environment_inner(&env_id, db.pool()).await.unwrap();
    assert!(list_environments_inner(db.pool()).await.unwrap().is_empty());
    match delete_environment_inner(&env_id, db.pool()).await {
        Err(RanchError::EnvironmentNotFound { .. }) => {}
        other => panic!("expected EnvironmentNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn delete_environment_refuses_when_it_holds_connections() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc = keychain();
    let conn = add_connection_inner(
        add_args(
            "redis",
            GroupSelector::named("Acme"),
            EnvironmentSelector::named("Local", "env-local"),
        ),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();

    match delete_environment_inner(&conn.environment_id, db.pool()).await {
        Err(RanchError::InvalidInput { reason }) => assert!(reason.contains("still contains 1")),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    assert_eq!(list_connections_inner(db.pool()).await.unwrap().len(), 1);
}

#[tokio::test]
async fn move_connection_across_groups_keeps_id_and_locks_on_production() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc = keychain();
    let conn = add_connection_inner(
        add_args(
            "redis",
            GroupSelector::named("Acme"),
            EnvironmentSelector::named("Local", "env-local"),
        ),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    let other = create_group_inner("Lumen", db.pool()).await.unwrap();
    let staging = create_environment_inner(&other.id, "Staging", "env-staging", db.pool())
        .await
        .unwrap();
    let prod = create_environment_inner(&other.id, "Production", "env-production", db.pool())
        .await
        .unwrap();

    // Plain move: other group, not production.
    let r = move_connection_inner(&conn.id, &staging, db.pool())
        .await
        .unwrap();
    assert!(!r.newly_locked);
    let row = &list_connections_inner(db.pool()).await.unwrap()[0];
    assert_eq!(row.id, conn.id);
    assert_eq!(row.group_name, "Lumen");
    assert_eq!(row.env_label, "Staging");
    assert!(!is_locked(&db, &conn.id).await);
    // The emptied environment stays as a drop target.
    assert_eq!(list_environments_inner(db.pool()).await.unwrap().len(), 3);

    // Into production: locked, and reported.
    let r = move_connection_inner(&conn.id, &prod, db.pool())
        .await
        .unwrap();
    assert!(r.newly_locked);
    assert!(is_locked(&db, &conn.id).await);

    // Back out: stays locked; same-environment move is a no-op.
    let r = move_connection_inner(&conn.id, &staging, db.pool())
        .await
        .unwrap();
    assert!(!r.newly_locked);
    assert!(is_locked(&db, &conn.id).await);
    let r = move_connection_inner(&conn.id, &staging, db.pool())
        .await
        .unwrap();
    assert!(!r.newly_locked);

    match move_connection_inner("nope", &staging, db.pool()).await {
        Err(RanchError::ConnectionNotFound { .. }) => {}
        other => panic!("expected ConnectionNotFound, got {other:?}"),
    }
    match move_connection_inner(&conn.id, "nope", db.pool()).await {
        Err(RanchError::EnvironmentNotFound { .. }) => {}
        other => panic!("expected EnvironmentNotFound, got {other:?}"),
    }
}
