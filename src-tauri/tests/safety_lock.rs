//! Integration tests for per-connection read-only lock.
//!
//! Covers:
//!   - Production-color connections default to locked at creation.
//!   - Non-production connections default to unlocked.
//!   - Actor's initial lock state matches the `connections.is_locked`
//!     column (the lazy spawn picks up the right value).
//!   - `unlock_connection` + `lock_connection` write the matching
//!     `unlockConnection` / `lockConnection` audit entries.
//!   - Restart re-locks a production connection (unlock state is
//!     in-memory only).
//!   - `check_lock_or_err` returns the typed `ConnectionLocked` error
//!     when called on `Locked`, `Ok(())` on `Unlocked`.
//!
//! That every MCP write tool refuses a locked connection before Redis is
//! `tests/lock_sentinel_live.rs`, which takes the list from the registry.

mod common;
use common::repo;

use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::commands::audit::{read_audit_log, ListAuditLogEntriesArgs};
use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::safety::{
    get_connection_lock_state_inner, lock_connection_inner, unlock_connection_inner, LockArgs,
};
use ranch_lib::contract::connections::ReadOnlyState as WireLock;
use ranch_lib::contract::errors::RanchErrorDto;
use ranch_lib::domain::audit::AuditAction;
use ranch_lib::domain::errors::RanchError;
use ranch_lib::domain::lock::{check_lock_or_err, ReadOnlyState};
use tempfile::tempdir;

fn add_args(name: &str, env_color: &str) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.to_string(),
        backend: "bullmq-redis".into(),
        endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
            host: "127.0.0.1".into(),
            port: 1,
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
        environment: EnvironmentSelector::for_color(env_color),
    }
}

async fn fresh_env() -> (
    tempfile::TempDir,
    Db,
    std::sync::Arc<dyn ranch_lib::application::ports::SecretStore>,
    ConnectionRegistry,
) {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());
    let registry = ConnectionRegistry::new();
    (dir, db, kc, registry)
}

// ─── Default lock state ──────────────────────────────────────────────────

#[tokio::test]
async fn prod_color_connection_starts_locked() {
    let (_d, db, kc, _r) = fresh_env().await;
    let conn = add_connection_inner(
        add_args("prod", "env-production"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let is_locked: i64 = sqlx::query_scalar("SELECT is_locked FROM connections WHERE id = ?")
        .bind(&conn.id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(is_locked, 1, "prod connection should default to locked");
}

#[tokio::test]
async fn non_prod_color_connection_starts_unlocked() {
    let (_d, db, kc, _r) = fresh_env().await;
    let conn = add_connection_inner(
        add_args("local", "env-local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let is_locked: i64 = sqlx::query_scalar("SELECT is_locked FROM connections WHERE id = ?")
        .bind(&conn.id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(
        is_locked, 0,
        "non-prod connection should default to unlocked"
    );
}

#[tokio::test]
async fn actor_initial_lock_state_matches_sqlite_for_prod() {
    let (_d, db, kc, reg) = fresh_env().await;
    let conn = add_connection_inner(
        add_args("prod", "env-production"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let state = get_connection_lock_state_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        &common::backends(db.pool(), &kc, &reg),
    )
    .await
    .unwrap();
    assert_eq!(state, WireLock::Locked);
}

#[tokio::test]
async fn actor_initial_lock_state_matches_sqlite_for_non_prod() {
    let (_d, db, kc, reg) = fresh_env().await;
    let conn = add_connection_inner(
        add_args("local", "env-local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let state = get_connection_lock_state_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        &common::backends(db.pool(), &kc, &reg),
    )
    .await
    .unwrap();
    assert_eq!(state, WireLock::Unlocked);
}

// ─── Toggle + audit ──────────────────────────────────────────────────────

#[tokio::test]
async fn unlock_writes_audit_entry() {
    let (_d, db, kc, reg) = fresh_env().await;
    let conn = add_connection_inner(
        add_args("prod", "env-production"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    let state = unlock_connection_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        db.pool(),
        &common::backends(db.pool(), &kc, &reg),
    )
    .await
    .unwrap();
    assert_eq!(state, WireLock::Unlocked);

    let entries = read_audit_log(
        ListAuditLogEntriesArgs {
            from: None,
            to: None,
            connection_id: Some(conn.id.clone()),
            action: Some("unlockConnection".into()),
            source: None,
            offset: 0,
            limit: 10,
        },
        db.pool(),
    )
    .await
    .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, AuditAction::UnlockConnection);
    assert_eq!(entries[0].connection_id.as_deref(), Some(conn.id.as_str()));
    assert_eq!(entries[0].env_color.as_deref(), Some("env-production"));
}

#[tokio::test]
async fn lock_writes_audit_entry_after_unlock() {
    let (_d, db, kc, reg) = fresh_env().await;
    let conn = add_connection_inner(
        add_args("prod", "env-production"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    // Unlock first so we have something to re-lock.
    unlock_connection_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        db.pool(),
        &common::backends(db.pool(), &kc, &reg),
    )
    .await
    .unwrap();

    let state = lock_connection_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        db.pool(),
        &common::backends(db.pool(), &kc, &reg),
    )
    .await
    .unwrap();
    assert_eq!(state, WireLock::Locked);

    let entries = read_audit_log(
        ListAuditLogEntriesArgs {
            from: None,
            to: None,
            connection_id: Some(conn.id.clone()),
            action: Some("lockConnection".into()),
            source: None,
            offset: 0,
            limit: 10,
        },
        db.pool(),
    )
    .await
    .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, AuditAction::LockConnection);
}

// ─── Restart re-locks (unlock state is in-memory only) ───────────────────

#[tokio::test]
async fn restart_relocks_prod_connection() {
    let (_d, db, kc, reg) = fresh_env().await;
    let conn = add_connection_inner(
        add_args("prod", "env-production"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    // Initial state: locked.
    let s0 = get_connection_lock_state_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        &common::backends(db.pool(), &kc, &reg),
    )
    .await
    .unwrap();
    assert_eq!(s0, WireLock::Locked);

    // Unlock for the session.
    unlock_connection_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        db.pool(),
        &common::backends(db.pool(), &kc, &reg),
    )
    .await
    .unwrap();
    let s1 = get_connection_lock_state_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        &common::backends(db.pool(), &kc, &reg),
    )
    .await
    .unwrap();
    assert_eq!(s1, WireLock::Unlocked);

    // Simulate app restart by shutting down the actor + acquiring fresh.
    reg.shutdown(&conn.id).await;
    let s2 = get_connection_lock_state_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        &common::backends(db.pool(), &kc, &reg),
    )
    .await
    .unwrap();
    assert_eq!(
        s2,
        WireLock::Locked,
        "actor restart must re-read is_locked from SQLite"
    );
}

// ─── check_lock_or_err helper. These duplicate the unit tests in
// `domain::lock::tests` but live here as part of the sentinel harness so
// the test file genuinely covers the lock surface end-to-end. ──────────

#[test]
fn check_lock_or_err_unlocked_passes() {
    let r = check_lock_or_err(ReadOnlyState::Unlocked, "conn-x", "Acme · prod");
    assert!(r.is_ok());
}

#[test]
fn check_lock_or_err_locked_returns_typed_error() {
    let err = check_lock_or_err(ReadOnlyState::Locked, "conn-x", "Acme · prod").unwrap_err();
    match err {
        RanchError::ConnectionLocked {
            connection_id,
            connection_name,
            unlock_hint,
        } => {
            assert_eq!(connection_id, "conn-x");
            assert_eq!(connection_name, "Acme · prod");
            assert!(unlock_hint.contains("cmd-shift-U"));
        }
        other => panic!("expected ConnectionLocked, got {other:?}"),
    }
}

#[test]
fn check_lock_or_err_locked_dto_has_kind_connection_locked() {
    let err = check_lock_or_err(ReadOnlyState::Locked, "conn-x", "Acme · prod").unwrap_err();
    let dto: RanchErrorDto = err.into();
    assert_eq!(dto.kind, "connectionLocked");
    assert!(dto.message.contains("Acme · prod"));
    assert!(dto.message.contains("cmd-shift-U"));
}

// ─── MCP destructive tools honor the connection lock ─────────

/// Helper: lock a connection by setting `is_locked = 1` in SQLite.
/// The actor reads this on first spawn.
async fn lock_connection_in_db(pool: &sqlx::SqlitePool, connection_id: &str) {
    sqlx::query("UPDATE connections SET is_locked = 1 WHERE id = ?")
        .bind(connection_id)
        .execute(pool)
        .await
        .unwrap();
}

async fn fresh_mcp_state() -> (
    tempfile::TempDir,
    ranch_lib::adapters::sqlite::Db,
    ranch_lib::shells::mcp::McpServerState,
    std::sync::Arc<dyn ranch_lib::application::ports::SecretStore>,
) {
    let dir = tempfile::tempdir().unwrap();
    let db = ranch_lib::adapters::sqlite::Db::init(dir.path())
        .await
        .unwrap();
    let keychain: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(ranch_lib::adapters::secrets::InMemoryKeychain::new());
    let registry = ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry::new();
    let state = common::mcp_state(
        db.pool(),
        std::sync::Arc::new(common::backends(db.pool(), &keychain, &registry)),
    );
    (dir, db, state, keychain)
}

fn mcp_job_action_input(
    connection_id: &str,
) -> rmcp::handler::server::wrapper::Parameters<ranch_lib::shells::mcp::server::JobActionInput> {
    rmcp::handler::server::wrapper::Parameters(ranch_lib::shells::mcp::server::JobActionInput {
        connection_id: connection_id.to_string(),
        queue_id: ranch_lib::shells::mcp::server::McpQueueId("bull:emails".into()),
        job_id: "42".into(),
    })
}

#[tokio::test]
async fn mcp_retry_job_on_locked_connection_returns_connection_locked() {
    let (_dir, db, state, keychain) = fresh_mcp_state().await;
    let conn = add_connection_inner(
        add_args("prod", "env-production"),
        &repo(db.pool()),
        &keychain,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    // Prod color → already starts locked, but be explicit defensively.
    lock_connection_in_db(db.pool(), &conn.id).await;

    let server = ranch_lib::shells::mcp::RanchMcpServer::new(state);
    let err = server
        .retry_job(mcp_job_action_input(&conn.id))
        .await
        .unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("locked") || msg.contains("Locked"),
        "expected ConnectionLocked message; got: {msg}"
    );
}

#[tokio::test]
async fn mcp_remove_job_on_locked_connection_returns_connection_locked() {
    let (_dir, db, state, keychain) = fresh_mcp_state().await;
    let conn = add_connection_inner(
        add_args("prod", "env-production"),
        &repo(db.pool()),
        &keychain,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    lock_connection_in_db(db.pool(), &conn.id).await;

    let server = ranch_lib::shells::mcp::RanchMcpServer::new(state);
    let err = server
        .remove_job(mcp_job_action_input(&conn.id))
        .await
        .unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("locked") || msg.contains("Locked"),
        "expected ConnectionLocked message; got: {msg}"
    );
}

#[tokio::test]
async fn mcp_promote_job_on_locked_connection_returns_connection_locked() {
    let (_dir, db, state, keychain) = fresh_mcp_state().await;
    let conn = add_connection_inner(
        add_args("prod", "env-production"),
        &repo(db.pool()),
        &keychain,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    lock_connection_in_db(db.pool(), &conn.id).await;

    let server = ranch_lib::shells::mcp::RanchMcpServer::new(state);
    let err = server
        .promote_job(mcp_job_action_input(&conn.id))
        .await
        .unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("locked") || msg.contains("Locked"),
        "expected ConnectionLocked message; got: {msg}"
    );
}

// "Read-only until I unlock it" is the user's choice when made.
#[tokio::test]
async fn an_explicit_lock_choice_wins_over_the_production_default() {
    let (_d, db, kc, _r) = fresh_env().await;
    let open_prod = add_connection_inner(
        AddConnectionArgs {
            locked: Some(false),
            ..add_args("prod", "env-production")
        },
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let locked_dev = add_connection_inner(
        AddConnectionArgs {
            locked: Some(true),
            ..add_args("dev", "env-local")
        },
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let lock_of = |id: String| {
        let pool = db.pool().clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT is_locked FROM connections WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(lock_of(open_prod.id).await, 0);
    assert_eq!(lock_of(locked_dev.id).await, 1);
}
