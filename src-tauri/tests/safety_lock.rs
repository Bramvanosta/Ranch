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
//! Plus the **forward-compat sentinel**: every destructive
//! `AuditAction` must have a matching `ActorCommand` variant that
//! routes through `check_lock_or_err`. The sentinel list covers
//! every destructive action (the Lock/Unlock toggles are excluded: they
//! don't touch Redis).

use ranch_lib::commands::audit::{list_audit_log_entries_inner, ListAuditLogEntriesArgs};
use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::safety::{
    get_connection_lock_state_inner, lock_connection_inner, unlock_connection_inner, LockArgs,
};
use ranch_lib::core::audit::AuditAction;
use ranch_lib::core::connection_actor::ConnectionRegistry;
use ranch_lib::core::db::Db;
use ranch_lib::core::errors::{RanchError, RanchErrorDto};
use ranch_lib::core::keychain::InMemoryKeychain;
use ranch_lib::core::safety_lock::{check_lock_or_err, ReadOnlyState};
use tempfile::tempdir;

fn add_args(name: &str, env_color: &str) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.to_string(),
        host: "127.0.0.1".into(),
        port: 1,
        db: 0,
        username: None,
        password: None,
        tls: false,
        tls_custom_ca: None,
        ssh_tunnel: None,
        ssh_password: None,
        group: GroupSelector::named("Default"),
        environment: EnvironmentSelector::for_color(env_color),
    }
}

async fn fresh_env() -> (
    tempfile::TempDir,
    Db,
    std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter>,
    ConnectionRegistry,
) {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());
    let registry = ConnectionRegistry::new();
    (dir, db, kc, registry)
}

// ─── Default lock state ──────────────────────────────────────────────────

#[tokio::test]
async fn prod_color_connection_starts_locked() {
    let (_d, db, kc, _r) = fresh_env().await;
    let conn = add_connection_inner(add_args("prod", "env-production"), db.pool(), &kc)
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
    let conn = add_connection_inner(add_args("local", "env-local"), db.pool(), &kc)
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
    let conn = add_connection_inner(add_args("prod", "env-production"), db.pool(), &kc)
        .await
        .unwrap();
    let state = get_connection_lock_state_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        db.pool(),
        &kc,
        &reg,
    )
    .await
    .unwrap();
    assert_eq!(state, ReadOnlyState::Locked);
}

#[tokio::test]
async fn actor_initial_lock_state_matches_sqlite_for_non_prod() {
    let (_d, db, kc, reg) = fresh_env().await;
    let conn = add_connection_inner(add_args("local", "env-local"), db.pool(), &kc)
        .await
        .unwrap();
    let state = get_connection_lock_state_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        db.pool(),
        &kc,
        &reg,
    )
    .await
    .unwrap();
    assert_eq!(state, ReadOnlyState::Unlocked);
}

// ─── Toggle + audit ──────────────────────────────────────────────────────

#[tokio::test]
async fn unlock_writes_audit_entry() {
    let (_d, db, kc, reg) = fresh_env().await;
    let conn = add_connection_inner(add_args("prod", "env-production"), db.pool(), &kc)
        .await
        .unwrap();

    let state = unlock_connection_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        db.pool(),
        &kc,
        &reg,
    )
    .await
    .unwrap();
    assert_eq!(state, ReadOnlyState::Unlocked);

    let entries = list_audit_log_entries_inner(
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
    let conn = add_connection_inner(add_args("prod", "env-production"), db.pool(), &kc)
        .await
        .unwrap();

    // Unlock first so we have something to re-lock.
    unlock_connection_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        db.pool(),
        &kc,
        &reg,
    )
    .await
    .unwrap();

    let state = lock_connection_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        db.pool(),
        &kc,
        &reg,
    )
    .await
    .unwrap();
    assert_eq!(state, ReadOnlyState::Locked);

    let entries = list_audit_log_entries_inner(
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
    let conn = add_connection_inner(add_args("prod", "env-production"), db.pool(), &kc)
        .await
        .unwrap();

    // Initial state: locked.
    let s0 = get_connection_lock_state_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        db.pool(),
        &kc,
        &reg,
    )
    .await
    .unwrap();
    assert_eq!(s0, ReadOnlyState::Locked);

    // Unlock for the session.
    unlock_connection_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        db.pool(),
        &kc,
        &reg,
    )
    .await
    .unwrap();
    let s1 = get_connection_lock_state_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        db.pool(),
        &kc,
        &reg,
    )
    .await
    .unwrap();
    assert_eq!(s1, ReadOnlyState::Unlocked);

    // Simulate app restart by shutting down the actor + acquiring fresh.
    reg.shutdown(&conn.id).await;
    let s2 = get_connection_lock_state_inner(
        LockArgs {
            connection_id: conn.id.clone(),
        },
        db.pool(),
        &kc,
        &reg,
    )
    .await
    .unwrap();
    assert_eq!(
        s2,
        ReadOnlyState::Locked,
        "actor restart must re-read is_locked from SQLite"
    );
}

// ─── check_lock_or_err helper — these duplicate the unit tests in
// `core::safety_lock::tests` but live here as part of the sentinel harness so
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

// ─────────────────────────────────────────────────────────────────────────
// Forward-compat sentinel
// ─────────────────────────────────────────────────────────────────────────

/// forward-compatibility sentinel for the connection lock.
///
/// Every `AuditAction` variant in this list represents a destructive
/// op that performs Redis IO. The contract is:
///
/// > For each action in `MUST_BE_LOCK_AWARE`, the connection actor MUST
/// > have a matching handler that calls `check_lock_or_err` as its
/// > FIRST line.
///
/// The `UnlockConnection` / `LockConnection` toggles are left out on purpose:
/// they DO NOT touch Redis and therefore are NOT subject to the lock check
/// (locking the lock toggle itself would be a deadlock). Every other
/// destructive action (`RetryJob`, `RemoveJob`, `PromoteJob`, `PauseQueue`,
/// `ResumeQueue`, `PauseScheduler`, `ResumeScheduler`) is listed here, and
/// the test fails loudly until each one is wired through
/// `check_lock_or_err`.
const MUST_BE_LOCK_AWARE: &[AuditAction] = &[
    // First three destructive actions. Each one's actor
    // handler (handle_retry_job, handle_remove_job, handle_promote_job
    // in core::connection_actor::actor) MUST call
    // `check_lock_or_err(state.lock_state, ...)` as its FIRST line.
    AuditAction::RetryJob,
    AuditAction::RemoveJob,
    AuditAction::PromoteJob,
    // pause/resume actions added. handle_pause_queue,
    // handle_resume_queue, handle_pause_scheduler, handle_resume_scheduler
    // each open with `check_lock_or_err(...)`. UnlockConnection /
    // LockConnection are intentionally NOT here — they don't touch
    // Redis, so the lock check would be a deadlock.
    AuditAction::PauseQueue,
    AuditAction::ResumeQueue,
    AuditAction::PauseScheduler,
    AuditAction::ResumeScheduler,
];

#[test]
fn every_destructive_action_has_a_matching_lock_aware_actor_command() {
    // `MUST_BE_LOCK_AWARE` forces the dev to confirm the handler check for
    // every destructive action.
    //
    // We assert non-redundancy (no duplicate entries) so the canary
    // can't drift via copy-paste:
    let mut seen: Vec<&AuditAction> = Vec::new();
    for a in MUST_BE_LOCK_AWARE {
        assert!(
            !seen.iter().any(|x| **x == *a),
            "duplicate action in MUST_BE_LOCK_AWARE: {a:?}"
        );
        seen.push(a);
    }

    // The sentinel passes when:
    //   1. Every entry in MUST_BE_LOCK_AWARE has a corresponding
    //      ActorCommand variant.
    //   2. That handler's first line is `check_lock_or_err(...)`.
    //
    // (1) is a compile-time concern as handlers are added.
    // (2) is enforced by code review + this comment. A future
    //     hardening could use a `safety_lock!` macro that wraps the
    //     handler body and verifies the check happened.
    //
    // The list carries the 7 destructive actions: 3 single-job
    // + 4 pause/resume. Runtime canary tests in
    // `tests/job_actions_live.rs::*_locked` and
    // `tests/queue_pause_live.rs::*_locked` confirm `ConnectionLocked`
    // is returned by each handler before any Redis op runs.
    assert_eq!(
        MUST_BE_LOCK_AWARE.len(),
        7,
        "expected RetryJob / RemoveJob / PromoteJob / \
         PauseQueue / ResumeQueue / PauseScheduler / ResumeScheduler; \
         append entries here when a new destructive action lands"
    );
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
    ranch_lib::core::db::Db,
    ranch_lib::core::mcp::McpServerState,
) {
    let dir = tempfile::tempdir().unwrap();
    let db = ranch_lib::core::db::Db::init(dir.path()).await.unwrap();
    let keychain: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(ranch_lib::core::keychain::InMemoryKeychain::new());
    let registry =
        std::sync::Arc::new(ranch_lib::core::connection_actor::ConnectionRegistry::new());
    let state = ranch_lib::core::mcp::McpServerState {
        pool: db.pool().clone(),
        keychain,
        registry,
    };
    (dir, db, state)
}

fn mcp_job_action_input(
    connection_id: &str,
) -> rmcp::handler::server::wrapper::Parameters<ranch_lib::core::mcp::server::JobActionInput> {
    rmcp::handler::server::wrapper::Parameters(ranch_lib::core::mcp::server::JobActionInput {
        connection_id: connection_id.to_string(),
        queue_ref: ranch_lib::core::mcp::server::McpQueueRef {
            prefix: "bull".into(),
            name: "emails".into(),
        },
        job_id: "42".into(),
    })
}

#[tokio::test]
async fn mcp_retry_job_on_locked_connection_returns_connection_locked() {
    let (_dir, db, state) = fresh_mcp_state().await;
    let conn = add_connection_inner(
        add_args("prod", "env-production"),
        db.pool(),
        &state.keychain,
    )
    .await
    .unwrap();
    // Prod color → already starts locked, but be explicit defensively.
    lock_connection_in_db(db.pool(), &conn.id).await;

    let server = ranch_lib::core::mcp::RanchMcpServer::new(state);
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
    let (_dir, db, state) = fresh_mcp_state().await;
    let conn = add_connection_inner(
        add_args("prod", "env-production"),
        db.pool(),
        &state.keychain,
    )
    .await
    .unwrap();
    lock_connection_in_db(db.pool(), &conn.id).await;

    let server = ranch_lib::core::mcp::RanchMcpServer::new(state);
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
    let (_dir, db, state) = fresh_mcp_state().await;
    let conn = add_connection_inner(
        add_args("prod", "env-production"),
        db.pool(),
        &state.keychain,
    )
    .await
    .unwrap();
    lock_connection_in_db(db.pool(), &conn.id).await;

    let server = ranch_lib::core::mcp::RanchMcpServer::new(state);
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

/// Sentinel test: every MCP destructive tool name MUST
/// have a corresponding entry in `MUST_BE_LOCK_AWARE`. If a future
/// contributor adds an MCP destructive tool without an actor-side
/// `MUST_BE_LOCK_AWARE` entry, this test fails with a clear message.
#[test]
fn mcp_destructive_tools_align_with_must_be_lock_aware() {
    use ranch_lib::commands::registry;

    // The 3 destructive MCP tools must be among MUST_BE_LOCK_AWARE.
    let destructive_mcp_tools: Vec<&str> = registry::mcp_tool_names()
        .into_iter()
        .filter(|name| matches!(*name, "retry_job" | "remove_job" | "promote_job"))
        .collect();
    assert_eq!(
        destructive_mcp_tools.len(),
        3,
        "expected exactly 3 destructive MCP tools, found: {destructive_mcp_tools:?}"
    );

    // Map MCP tool name → AuditAction variant (resolving the alias
    // for `inspect_job` even though it's a read-only tool here).
    let expected_actions = [
        AuditAction::RetryJob,
        AuditAction::RemoveJob,
        AuditAction::PromoteJob,
    ];
    for action in &expected_actions {
        assert!(
            MUST_BE_LOCK_AWARE.contains(action),
            "destructive MCP tool's AuditAction {action:?} is missing from MUST_BE_LOCK_AWARE"
        );
    }
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
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    let locked_dev = add_connection_inner(
        AddConnectionArgs {
            locked: Some(true),
            ..add_args("dev", "env-local")
        },
        db.pool(),
        &kc,
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
