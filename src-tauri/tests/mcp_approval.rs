//! MCP write approval.
//!
//! `ranch-mcp` and the GUI are two processes sharing one SQLite file. These
//! tests reproduce that with **two pools on the same database**: the "MCP"
//! pool enqueues and polls, the "GUI" pool beats, lists and decides.
//!
//! The write tools are pointed at `127.0.0.1:1`, so "got past the gate" shows
//! up as a Redis connection error — no live Redis needed to prove who was
//! let through and who was not.

use std::sync::Arc;
use std::time::Duration;

use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::mcp_safety::{
    get_mcp_safety_settings_inner, set_connection_mcp_readonly_inner,
    set_mcp_safety_settings_inner, McpSafetySettings,
};
use ranch_lib::core::db::Db;
use ranch_lib::core::errors::RanchError;
use ranch_lib::core::keychain::{InMemoryKeychain, KeychainAdapter};
use ranch_lib::core::mcp::approval::{
    self, ApprovalDecision, ApprovalMode, DecideOutcome, NewApproval, PollOutcome,
};
use ranch_lib::core::mcp::gate::{self, GateRequest, ToolKind};
use ranch_lib::core::mcp::heartbeat::{
    clear_heartbeat, is_gui_alive, write_heartbeat, HEARTBEAT_KEY,
};
use sqlx::SqlitePool;
use tempfile::tempdir;

const FAST_POLL: Duration = Duration::from_millis(10);

struct Env {
    _dir: tempfile::TempDir,
    /// The pool `ranch-mcp` would hold.
    mcp: SqlitePool,
    /// The pool the GUI would hold — a second `Db::init` on the same file.
    gui: SqlitePool,
    connection_id: String,
}

async fn env() -> Env {
    let dir = tempdir().unwrap();
    let mcp_db = Db::init(dir.path()).await.unwrap();
    let gui_db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn KeychainAdapter> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "queues".into(),
            host: "127.0.0.1".into(),
            port: 1,
            db: 0,
            username: None,
            password: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
            ssh_password: None,
            group: GroupSelector::named("Lumen"),
            environment: EnvironmentSelector::named("Staging", "env-staging"),
        },
        gui_db.pool(),
        &kc,
    )
    .await
    .unwrap();
    Env {
        _dir: dir,
        mcp: mcp_db.pool().clone(),
        gui: gui_db.pool().clone(),
        connection_id: conn.id,
    }
}

fn request(env: &Env, tool: &str, kind: ToolKind) -> GateRequest {
    GateRequest {
        tool: tool.into(),
        kind,
        connection_id: env.connection_id.clone(),
        client_hint: Some("claude-desktop".into()),
        arguments: serde_json::json!({ "jobId": "42" }),
        target_preview: Some(serde_json::json!({ "kind": "job", "name": "send-email" })),
    }
}

fn new_approval(env: &Env, tool: &str) -> NewApproval {
    NewApproval {
        client_hint: Some("claude-desktop".into()),
        connection_id: env.connection_id.clone(),
        tool: tool.into(),
        kind: ToolKind::Write,
        arguments: serde_json::json!({}),
        target_preview: None,
    }
}

/// The GUI answers the first pending request it sees, from its own pool.
fn gui_decides(gui: SqlitePool, decision: ApprovalDecision, reason: Option<&'static str>) {
    tokio::spawn(async move {
        loop {
            let pending = approval::list_pending(&gui).await.unwrap();
            if let Some(first) = pending.first() {
                approval::decide(&gui, &first.id, decision, reason)
                    .await
                    .unwrap();
                return;
            }
            tokio::time::sleep(FAST_POLL).await;
        }
    });
}

// ── settings ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn defaults_are_writes_only_and_120_seconds() {
    let e = env().await;
    let s = get_mcp_safety_settings_inner(&e.gui).await.unwrap();
    assert_eq!(s.approval_mode, ApprovalMode::WritesOnly);
    assert_eq!(s.approval_timeout_seconds, 120);
    assert!(!s.readonly_default);
}

#[tokio::test]
async fn settings_round_trip_and_the_timeout_is_clamped() {
    let e = env().await;
    let saved = set_mcp_safety_settings_inner(
        &McpSafetySettings {
            readonly_default: true,
            approval_mode: ApprovalMode::All,
            approval_timeout_seconds: 1,
            approval_always_on_top: false,
            approval_notify_sound: true,
        },
        &e.gui,
    )
    .await
    .unwrap();
    assert_eq!(saved.approval_mode, ApprovalMode::All);
    assert_eq!(saved.approval_timeout_seconds, 10, "clamped to the minimum");
    // …and the other process sees it.
    assert_eq!(get_mcp_safety_settings_inner(&e.mcp).await.unwrap(), saved);
}

#[test]
fn an_unreadable_mode_never_means_off() {
    assert_eq!(ApprovalMode::parse(None), ApprovalMode::WritesOnly);
    assert_eq!(
        ApprovalMode::parse(Some("garbage")),
        ApprovalMode::WritesOnly
    );
    assert_eq!(ApprovalMode::parse(Some("off")), ApprovalMode::Off);
    assert!(ApprovalMode::WritesOnly.requires_approval(ToolKind::Write));
    assert!(!ApprovalMode::WritesOnly.requires_approval(ToolKind::Read));
    assert!(ApprovalMode::All.requires_approval(ToolKind::Read));
    assert!(!ApprovalMode::Off.requires_approval(ToolKind::Write));
}

// ── heartbeat ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn heartbeat_is_visible_across_processes_and_goes_stale() {
    let e = env().await;
    assert!(!is_gui_alive(&e.mcp).await.unwrap(), "no beat yet");

    write_heartbeat(&e.gui).await.unwrap();
    assert!(is_gui_alive(&e.mcp).await.unwrap());

    // 16 s old → stale. A beat from the future is no proof of life either.
    for offset in [-16_i64, 3600] {
        let at = (chrono::Utc::now() + chrono::Duration::seconds(offset)).to_rfc3339();
        sqlx::query("UPDATE kv_settings SET value = ? WHERE key = ?")
            .bind(at)
            .bind(HEARTBEAT_KEY)
            .execute(&e.gui)
            .await
            .unwrap();
        assert!(!is_gui_alive(&e.mcp).await.unwrap(), "offset {offset}s");
    }

    write_heartbeat(&e.gui).await.unwrap();
    clear_heartbeat(&e.gui).await.unwrap();
    assert!(!is_gui_alive(&e.mcp).await.unwrap(), "clean exit clears it");
}

// ── the gate ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn app_closed_fails_fast_without_queuing_anything() {
    let e = env().await;
    let started = std::time::Instant::now();
    let err = gate::authorize(&e.mcp, request(&e, "remove_job", ToolKind::Write))
        .await
        .unwrap_err();
    assert!(matches!(err, RanchError::HostUnavailable { .. }), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(2), "must not wait");
    let (rows,): (i64,) = sqlx::query_as("SELECT count(*) FROM mcp_pending_approvals")
        .fetch_one(&e.gui)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn approve_returns_a_grant_and_the_gui_saw_the_request_as_it_should_display_it() {
    let e = env().await;
    write_heartbeat(&e.gui).await.unwrap();

    let gui = e.gui.clone();
    let seen = tokio::spawn(async move {
        loop {
            let pending = approval::list_pending(&gui).await.unwrap();
            if let Some(first) = pending.first().cloned() {
                approval::decide(&gui, &first.id, ApprovalDecision::Approve, None)
                    .await
                    .unwrap();
                return first;
            }
            tokio::time::sleep(FAST_POLL).await;
        }
    });

    let grant = gate::authorize(&e.mcp, request(&e, "remove_job", ToolKind::Write))
        .await
        .unwrap()
        .expect("writes_only → a write needs a grant");
    assert_eq!(grant.tool(), "remove_job");

    let shown = seen.await.unwrap();
    assert_eq!(shown.id, grant.approval_id());
    assert_eq!(shown.client_hint.as_deref(), Some("claude-desktop"));
    assert_eq!(shown.connection_name, "queues");
    assert_eq!(shown.group_name, "Lumen");
    assert_eq!(shown.environment_name, "Staging");
    assert_eq!(shown.env_color, "env-staging");
    assert_eq!(shown.kind, "write");
    assert_eq!(shown.timeout_seconds, 120);
    assert!(shown.target_preview_json.unwrap().contains("send-email"));
}

#[tokio::test]
async fn deny_reaches_the_agent_with_the_reason() {
    let e = env().await;
    write_heartbeat(&e.gui).await.unwrap();
    gui_decides(e.gui.clone(), ApprovalDecision::Deny, Some("  not now  "));

    let err = gate::authorize(&e.mcp, request(&e, "retry_job", ToolKind::Write))
        .await
        .unwrap_err();
    match err {
        RanchError::ApprovalDenied { tool, reason } => {
            assert_eq!(tool, "retry_job");
            assert_eq!(reason.as_deref(), Some("not now"), "trimmed");
        }
        other => panic!("expected ApprovalDenied, got {other:?}"),
    }
}

#[tokio::test]
async fn reads_are_free_unless_the_mode_is_all_and_off_asks_nothing() {
    let e = env().await;
    // writes_only + no GUI: a read passes, untouched by the heartbeat.
    assert!(
        gate::authorize(&e.mcp, request(&e, "list_jobs", ToolKind::Read))
            .await
            .unwrap()
            .is_none()
    );

    let mut s = get_mcp_safety_settings_inner(&e.gui).await.unwrap();
    s.approval_mode = ApprovalMode::All;
    set_mcp_safety_settings_inner(&s, &e.gui).await.unwrap();
    let err = gate::authorize(&e.mcp, request(&e, "list_jobs", ToolKind::Read))
        .await
        .unwrap_err();
    assert!(
        matches!(err, RanchError::HostUnavailable { .. }),
        "all → reads wait too"
    );

    s.approval_mode = ApprovalMode::Off;
    set_mcp_safety_settings_inner(&s, &e.gui).await.unwrap();
    assert!(
        gate::authorize(&e.mcp, request(&e, "remove_job", ToolKind::Write))
            .await
            .unwrap()
            .is_none(),
        "off → no grant; the session lock decides"
    );
}

#[tokio::test]
async fn mcp_readonly_wins_over_everything_else() {
    let e = env().await;
    write_heartbeat(&e.gui).await.unwrap();
    set_connection_mcp_readonly_inner(&e.connection_id, Some(true), &e.gui)
        .await
        .unwrap();
    let err = gate::authorize(&e.mcp, request(&e, "remove_job", ToolKind::Write))
        .await
        .unwrap_err();
    assert!(matches!(err, RanchError::BlockedReadonly { .. }), "{err:?}");
    assert!(
        approval::list_pending(&e.gui).await.unwrap().is_empty(),
        "nobody is asked"
    );
}

// ── polling: timeout, vanished GUI, races ───────────────────────────────────

#[tokio::test]
async fn nobody_answers_times_out_and_expires_the_request() {
    let e = env().await;
    write_heartbeat(&e.gui).await.unwrap();
    let id = approval::enqueue(&e.mcp, &new_approval(&e, "remove_job"))
        .await
        .unwrap();
    let outcome = approval::poll_decision(&e.mcp, &id, Duration::from_millis(60), FAST_POLL)
        .await
        .unwrap();
    assert_eq!(outcome, PollOutcome::TimedOut);
    assert!(approval::list_pending(&e.gui).await.unwrap().is_empty());
    // Too late now.
    assert_eq!(
        approval::decide(&e.gui, &id, ApprovalDecision::Approve, None)
            .await
            .unwrap(),
        DecideOutcome::AlreadyDecided
    );
}

#[tokio::test]
async fn gui_closing_while_waiting_is_reported_as_host_unavailable() {
    let e = env().await;
    write_heartbeat(&e.gui).await.unwrap();
    let id = approval::enqueue(&e.mcp, &new_approval(&e, "remove_job"))
        .await
        .unwrap();

    let gui = e.gui.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(40)).await;
        clear_heartbeat(&gui).await.unwrap();
    });
    let outcome = approval::poll_decision(&e.mcp, &id, Duration::from_secs(30), FAST_POLL)
        .await
        .unwrap();
    assert_eq!(outcome, PollOutcome::HostUnavailable);
}

#[tokio::test]
async fn a_decision_is_single_writer() {
    let e = env().await;
    let id = approval::enqueue(&e.mcp, &new_approval(&e, "remove_job"))
        .await
        .unwrap();
    assert_eq!(
        approval::decide(&e.gui, &id, ApprovalDecision::Deny, None)
            .await
            .unwrap(),
        DecideOutcome::Decided
    );
    // A second click, another window, a replay: no-ops.
    assert_eq!(
        approval::decide(&e.gui, &id, ApprovalDecision::Approve, None)
            .await
            .unwrap(),
        DecideOutcome::AlreadyDecided
    );
    assert_eq!(
        approval::decide(&e.gui, "no-such-id", ApprovalDecision::Approve, None)
            .await
            .unwrap(),
        DecideOutcome::AlreadyDecided
    );
    // The agent cannot expire what the human already decided.
    assert!(!approval::expire(&e.mcp, &id).await.unwrap());
}

// ── What a grant is worth ───────────────────────────────────────────────────

#[tokio::test]
async fn consume_works_once_and_only_for_the_approved_connection_and_tool() {
    let e = env().await;
    let id = approval::enqueue(&e.mcp, &new_approval(&e, "remove_job"))
        .await
        .unwrap();

    // Pending: worth nothing yet.
    assert!(
        !approval::consume(&e.mcp, &id, &e.connection_id, "remove_job")
            .await
            .unwrap()
    );
    approval::decide(&e.gui, &id, ApprovalDecision::Approve, None)
        .await
        .unwrap();

    // Approved for remove_job on this connection — not for anything else.
    assert!(
        !approval::consume(&e.mcp, &id, &e.connection_id, "retry_job")
            .await
            .unwrap()
    );
    assert!(
        !approval::consume(&e.mcp, &id, "another-connection", "remove_job")
            .await
            .unwrap()
    );
    assert!(
        !approval::consume(&e.mcp, "forged-id", &e.connection_id, "remove_job")
            .await
            .unwrap()
    );

    assert!(
        approval::consume(&e.mcp, &id, &e.connection_id, "remove_job")
            .await
            .unwrap()
    );
    // Replay.
    assert!(
        !approval::consume(&e.mcp, &id, &e.connection_id, "remove_job")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn denied_and_expired_requests_cannot_be_consumed() {
    let e = env().await;
    let denied = approval::enqueue(&e.mcp, &new_approval(&e, "remove_job"))
        .await
        .unwrap();
    approval::decide(&e.gui, &denied, ApprovalDecision::Deny, None)
        .await
        .unwrap();
    let expired = approval::enqueue(&e.mcp, &new_approval(&e, "remove_job"))
        .await
        .unwrap();
    assert!(approval::expire(&e.mcp, &expired).await.unwrap());

    for id in [&denied, &expired] {
        assert!(
            !approval::consume(&e.mcp, id, &e.connection_id, "remove_job")
                .await
                .unwrap()
        );
    }
}

// ── Purging of old requests ─────────────────────────────────────────────────

#[tokio::test]
async fn requests_older_than_an_hour_are_purged_whatever_their_status() {
    let e = env().await;
    let old = approval::enqueue(&e.mcp, &new_approval(&e, "remove_job"))
        .await
        .unwrap();
    let fresh = approval::enqueue(&e.mcp, &new_approval(&e, "remove_job"))
        .await
        .unwrap();
    let two_hours_ago = (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
    sqlx::query("UPDATE mcp_pending_approvals SET created_at = ? WHERE id = ?")
        .bind(two_hours_ago)
        .bind(&old)
        .execute(&e.gui)
        .await
        .unwrap();

    assert_eq!(approval::purge_old(&e.gui).await.unwrap(), 1);
    let left = approval::list_pending(&e.gui).await.unwrap();
    assert_eq!(
        left.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec![fresh.as_str()]
    );

    // A purged request reads as "nobody can approve this anymore".
    let outcome = approval::poll_decision(&e.mcp, &old, Duration::from_secs(5), FAST_POLL)
        .await
        .unwrap();
    assert_eq!(outcome, PollOutcome::TimedOut);
}

// ── Approval at the enforcement point: the connection actor ─────────────────
//
// `authorize_write` runs before the actor opens Redis, so with Redis at
// 127.0.0.1:1 the outcome tells exactly how far a call got:
//   ConnectionLocked / ApprovalInvalid → stopped by the authorization;
//   a Redis error                      → authorized, then failed to connect.

use ranch_lib::commands::connections::resolve_connection_config;
use ranch_lib::commands::jobs::JobActionArgs;
use ranch_lib::core::bullmq_schema::QueueRef;
use ranch_lib::core::connection_actor::{AuditAttribution, ConnectionRegistry};
use ranch_lib::core::mcp::gate::ApprovalGrant;

struct ActorEnv {
    env: Env,
    registry: ConnectionRegistry,
    kc: Arc<dyn KeychainAdapter>,
    /// A Production connection: created locked.
    locked_id: String,
}

async fn actor_env() -> ActorEnv {
    let e = env().await;
    let kc: Arc<dyn KeychainAdapter> = Arc::new(InMemoryKeychain::new());
    let locked = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "prod-queues".into(),
            host: "127.0.0.1".into(),
            port: 1,
            db: 0,
            username: None,
            password: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
            ssh_password: None,
            group: GroupSelector::named("Lumen"),
            environment: EnvironmentSelector::named("Production", "env-production"),
        },
        &e.gui,
        &kc,
    )
    .await
    .unwrap();
    ActorEnv {
        env: e,
        registry: ConnectionRegistry::new(),
        kc,
        locked_id: locked.id,
    }
}

async fn retry_on_locked(a: &ActorEnv, attribution: AuditAttribution) -> RanchError {
    let config = resolve_connection_config(&a.locked_id, &a.env.mcp, &a.kc)
        .await
        .unwrap();
    a.registry
        .retry_job(
            &a.locked_id,
            &config,
            JobActionArgs {
                connection_id: a.locked_id.clone(),
                queue_ref: QueueRef {
                    prefix: "bull".into(),
                    name: "emails".into(),
                },
                job_id: "42".into(),
            },
            attribution,
        )
        .await
        .expect("actor alive")
        .expect_err("Redis is unreachable, so this can never succeed")
}

async fn approved_request(a: &ActorEnv, tool: &str) -> String {
    let id = approval::enqueue(
        &a.env.mcp,
        &NewApproval {
            client_hint: None,
            connection_id: a.locked_id.clone(),
            tool: tool.into(),
            kind: ToolKind::Write,
            arguments: serde_json::json!({}),
            target_preview: None,
        },
    )
    .await
    .unwrap();
    approval::decide(&a.env.gui, &id, ApprovalDecision::Approve, None)
        .await
        .unwrap();
    id
}

async fn status_of(pool: &SqlitePool, id: &str) -> String {
    let (s,): (String,) = sqlx::query_as("SELECT status FROM mcp_pending_approvals WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
    s
}

/// (c) No approval → the lock applies to agents exactly as without the approval flow.
#[tokio::test]
async fn without_a_grant_a_locked_connection_rejects_the_agent() {
    let a = actor_env().await;
    let err = retry_on_locked(&a, AuditAttribution::agent("claude-desktop", "stdio")).await;
    assert!(
        matches!(err, RanchError::ConnectionLocked { .. }),
        "{err:?}"
    );
}

/// (b) A human approval authorizes this one operation even though the
/// connection is session-locked — and is used up by it.
#[tokio::test]
async fn an_approved_grant_gets_past_the_lock_exactly_once() {
    let a = actor_env().await;
    let id = approved_request(&a, "retry_job").await;
    let grant = ApprovalGrant::forge_for_test(&id, "retry_job");

    let err = retry_on_locked(
        &a,
        AuditAttribution::agent("claude-desktop", "stdio").with_approval(Some(grant.clone())),
    )
    .await;
    assert!(
        !matches!(
            err,
            RanchError::ConnectionLocked { .. } | RanchError::ApprovalInvalid { .. }
        ),
        "expected to get past the authorization and fail on Redis, got {err:?}"
    );
    assert_eq!(status_of(&a.env.gui, &id).await, "consumed");

    // Replay of the same grant: worthless.
    let err = retry_on_locked(
        &a,
        AuditAttribution::agent("claude-desktop", "stdio").with_approval(Some(grant)),
    )
    .await;
    assert!(matches!(err, RanchError::ApprovalInvalid { .. }), "{err:?}");
}

/// (d) Forged, pending, denied, expired, wrong-tool: none of them authorizes
/// anything — and none of them falls back to "well, then check the lock".
#[tokio::test]
async fn invalid_grants_never_authorize() {
    let a = actor_env().await;

    let pending = approval::enqueue(
        &a.env.mcp,
        &NewApproval {
            client_hint: None,
            connection_id: a.locked_id.clone(),
            tool: "retry_job".into(),
            kind: ToolKind::Write,
            arguments: serde_json::json!({}),
            target_preview: None,
        },
    )
    .await
    .unwrap();
    let denied = approved_request(&a, "retry_job").await;
    sqlx::query("UPDATE mcp_pending_approvals SET status = 'denied' WHERE id = ?")
        .bind(&denied)
        .execute(&a.env.gui)
        .await
        .unwrap();
    let expired = approved_request(&a, "retry_job").await;
    sqlx::query("UPDATE mcp_pending_approvals SET status = 'expired' WHERE id = ?")
        .bind(&expired)
        .execute(&a.env.gui)
        .await
        .unwrap();
    let for_remove = approved_request(&a, "remove_job").await;

    for (label, grant) in [
        (
            "forged",
            ApprovalGrant::forge_for_test("not-a-real-id", "retry_job"),
        ),
        (
            "pending",
            ApprovalGrant::forge_for_test(&pending, "retry_job"),
        ),
        (
            "denied",
            ApprovalGrant::forge_for_test(&denied, "retry_job"),
        ),
        (
            "expired",
            ApprovalGrant::forge_for_test(&expired, "retry_job"),
        ),
        // Approved — for another tool. Both spellings of the lie are refused.
        (
            "wrong tool (honest grant)",
            ApprovalGrant::forge_for_test(&for_remove, "remove_job"),
        ),
        (
            "wrong tool (relabelled)",
            ApprovalGrant::forge_for_test(&for_remove, "retry_job"),
        ),
    ] {
        let err = retry_on_locked(
            &a,
            AuditAttribution::agent("claude-desktop", "stdio").with_approval(Some(grant)),
        )
        .await;
        assert!(
            matches!(err, RanchError::ApprovalInvalid { .. }),
            "{label}: expected ApprovalInvalid, got {err:?}"
        );
    }
    // The genuine remove_job approval was not burnt by the attempts.
    assert_eq!(status_of(&a.env.gui, &for_remove).await, "approved");
}

// ── The same three answers on the handlers that became agent-reachable ──

use ranch_lib::commands::jobs::BulkActionArgs;
use ranch_lib::commands::queues::QueueActionArgs;

fn emails() -> QueueRef {
    QueueRef {
        prefix: "bull".into(),
        name: "emails".into(),
    }
}

async fn pause_on_locked(a: &ActorEnv, attribution: AuditAttribution) -> RanchError {
    let config = resolve_connection_config(&a.locked_id, &a.env.mcp, &a.kc)
        .await
        .unwrap();
    a.registry
        .pause_queue_as(
            &a.locked_id,
            &config,
            QueueActionArgs {
                connection_id: a.locked_id.clone(),
                queue_ref: emails(),
            },
            attribution,
        )
        .await
        .expect("actor alive")
        .expect_err("Redis is unreachable")
}

async fn bulk_remove_on_locked(a: &ActorEnv, attribution: AuditAttribution) -> RanchError {
    let config = resolve_connection_config(&a.locked_id, &a.env.mcp, &a.kc)
        .await
        .unwrap();
    a.registry
        .bulk_remove_jobs_as(
            &a.locked_id,
            &config,
            BulkActionArgs {
                connection_id: a.locked_id.clone(),
                queue_ref: emails(),
                job_ids: vec!["1".into(), "2".into()],
            },
            attribution,
        )
        .await
        .expect("actor alive")
        .expect_err("Redis is unreachable")
}

#[tokio::test]
async fn pause_and_bulk_handlers_enforce_lock_approval_and_invalid_grants() {
    let a = actor_env().await;
    let agent = || AuditAttribution::agent("claude-desktop", "stdio");

    // No grant → the lock.
    assert!(matches!(
        pause_on_locked(&a, agent()).await,
        RanchError::ConnectionLocked { .. }
    ));
    assert!(matches!(
        bulk_remove_on_locked(&a, agent()).await,
        RanchError::ConnectionLocked { .. }
    ));

    // Forged grant → invalid, not "then check the lock".
    let forged =
        |tool: &str| agent().with_approval(Some(ApprovalGrant::forge_for_test("nope", tool)));
    assert!(matches!(
        pause_on_locked(&a, forged("pause_queue")).await,
        RanchError::ApprovalInvalid { .. }
    ));
    assert!(matches!(
        bulk_remove_on_locked(&a, forged("bulk_remove_jobs")).await,
        RanchError::ApprovalInvalid { .. }
    ));

    // A single-job approval cannot be spent on the bulk tool.
    let single = approved_request(&a, "remove_job").await;
    let err = bulk_remove_on_locked(
        &a,
        agent().with_approval(Some(ApprovalGrant::forge_for_test(
            &single,
            "bulk_remove_jobs",
        ))),
    )
    .await;
    assert!(matches!(err, RanchError::ApprovalInvalid { .. }), "{err:?}");
    assert_eq!(status_of(&a.env.gui, &single).await, "approved");

    // Genuine approvals get past the lock (and are consumed).
    for (tool, is_pause) in [("pause_queue", true), ("bulk_remove_jobs", false)] {
        let id = approved_request(&a, tool).await;
        let attribution = agent().with_approval(Some(ApprovalGrant::forge_for_test(&id, tool)));
        let err = if is_pause {
            pause_on_locked(&a, attribution).await
        } else {
            bulk_remove_on_locked(&a, attribution).await
        };
        assert!(
            !matches!(
                err,
                RanchError::ConnectionLocked { .. } | RanchError::ApprovalInvalid { .. }
            ),
            "{tool}: expected to pass the authorization, got {err:?}"
        );
        assert_eq!(status_of(&a.env.gui, &id).await, "consumed", "{tool}");
    }
}

// A decided request outlives the hour: the audit log reads it.
#[tokio::test]
async fn a_decided_request_is_kept_for_the_audit_log() {
    let e = env().await;
    let id = approval::enqueue(&e.mcp, &new_approval(&e, "remove_job"))
        .await
        .unwrap();
    approval::decide(&e.gui, &id, approval::ApprovalDecision::Approve, None)
        .await
        .unwrap();
    let two_hours_ago = (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
    sqlx::query("UPDATE mcp_pending_approvals SET created_at = ? WHERE id = ?")
        .bind(two_hours_ago)
        .bind(&id)
        .execute(&e.gui)
        .await
        .unwrap();
    assert_eq!(approval::purge_old(&e.gui).await.unwrap(), 0);
    let record = approval::record(&e.gui, &id).await.unwrap().expect("kept");
    assert_eq!(record.status, "approved");
    assert!(record.decided_at.is_some());
    assert_eq!(record.tool, "remove_job");
    assert!(approval::record(&e.gui, "nope").await.unwrap().is_none());
}
