//! MCP safety gate.
//!
//! Step 1 of the gate: a connection that is read-only for agents rejects
//! every MCP **write** tool with `blockedReadonly`, before anything reaches
//! the connection actor (no Redis needed for these tests — that is the
//! point). Read tools are never restricted.

use std::sync::Arc;

use ranch_lib::commands::connections::{
    add_connection_inner, list_connections_inner, AddConnectionArgs,
};
use ranch_lib::commands::hierarchy::{
    change_environment_color_inner, create_environment_inner, move_connection_inner,
    EnvironmentSelector, GroupSelector,
};
use ranch_lib::commands::mcp_safety::{
    get_mcp_safety_settings_inner, set_connection_mcp_readonly_inner,
    set_mcp_safety_settings_inner, McpSafetySettings,
};
use ranch_lib::commands::registry;
use ranch_lib::core::connection_actor::ConnectionRegistry;
use ranch_lib::core::db::Db;
use ranch_lib::core::keychain::{InMemoryKeychain, KeychainAdapter};
use ranch_lib::core::mcp::approval::ApprovalMode;
use ranch_lib::core::mcp::gate::{effective_mcp_readonly, ToolKind};
use ranch_lib::core::mcp::server::{JobActionInput, McpQueueRef};
use ranch_lib::core::mcp::{McpServerState, RanchMcpServer};
use rmcp::handler::server::wrapper::Parameters;
use tempfile::tempdir;

async fn fresh() -> (
    tempfile::TempDir,
    Db,
    RanchMcpServer,
    Arc<dyn KeychainAdapter>,
) {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let keychain: Arc<dyn KeychainAdapter> = Arc::new(InMemoryKeychain::new());
    let server = RanchMcpServer::new(McpServerState {
        pool: db.pool().clone(),
        keychain: keychain.clone(),
        registry: Arc::new(ConnectionRegistry::new()),
    });
    (dir, db, server, keychain)
}

fn add_args(name: &str, env_name: &str, color: &str) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.into(),
        // Nothing listens here: a call that gets PAST the gate fails fast on
        // the connection, which is how the tests tell "blocked" from "let through".
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
        environment: EnvironmentSelector::named(env_name, color),
    }
}

fn job_input(connection_id: &str) -> Parameters<JobActionInput> {
    Parameters(JobActionInput {
        connection_id: connection_id.into(),
        queue_ref: McpQueueRef {
            prefix: "bull".into(),
            name: "emails".into(),
        },
        job_id: "42".into(),
    })
}

/// `data.kind` of an MCP error, the machine-readable discriminant.
fn kind_of(err: &rmcp::ErrorData) -> String {
    err.data
        .as_ref()
        .and_then(|d| d.get("kind"))
        .and_then(|k| k.as_str())
        .unwrap_or("<none>")
        .to_string()
}

/// Call every write tool on `connection_id` and return the error kinds.
async fn write_tool_error_kinds(
    server: &RanchMcpServer,
    connection_id: &str,
) -> Vec<(String, String)> {
    use ranch_lib::core::mcp::server::{
        AddJobInput, AnnotateCauseInput, BulkJobsInput, BulkRetryInput, CleanQueueInput,
        DrainQueueInput, QueueInput, SchedulerInput, UpsertSchedulerInput,
    };
    let queue_ref = || McpQueueRef {
        prefix: "bull".into(),
        name: "emails".into(),
    };
    let queue = || {
        Parameters(QueueInput {
            connection_id: connection_id.into(),
            queue_ref: queue_ref(),
        })
    };
    let scheduler = || {
        Parameters(SchedulerInput {
            connection_id: connection_id.into(),
            queue_ref: queue_ref(),
            scheduler_key: "nightly".into(),
        })
    };
    let bulk = || {
        Parameters(BulkJobsInput {
            connection_id: connection_id.into(),
            queue_ref: queue_ref(),
            job_ids: vec!["1".into(), "2".into()],
        })
    };

    let mut out = Vec::new();
    macro_rules! record {
        ($name:literal, $call:expr) => {
            out.push(($name.to_string(), kind_of(&$call.await.unwrap_err())));
        };
    }
    record!("retry_job", server.retry_job(job_input(connection_id)));
    record!("remove_job", server.remove_job(job_input(connection_id)));
    record!("promote_job", server.promote_job(job_input(connection_id)));
    record!(
        "bulk_retry_jobs",
        server.bulk_retry_jobs(Parameters(BulkRetryInput {
            connection_id: connection_id.into(),
            queue_ref: queue_ref(),
            job_ids: vec!["1".into(), "2".into()],
            all_failed: false,
        }))
    );
    // The all-failed selector is the same tool: same gate, same answer.
    let all_failed = server
        .bulk_retry_jobs(Parameters(BulkRetryInput {
            connection_id: connection_id.into(),
            queue_ref: queue_ref(),
            job_ids: vec![],
            all_failed: true,
        }))
        .await
        .unwrap_err();
    assert_eq!(
        kind_of(&all_failed),
        out.last().expect("bulk_retry_jobs recorded").1,
        "allFailed must go through the same gate as jobIds"
    );
    record!("bulk_remove_jobs", server.bulk_remove_jobs(bulk()));
    record!("bulk_promote_jobs", server.bulk_promote_jobs(bulk()));
    record!("pause_queue", server.pause_queue(queue()));
    record!("resume_queue", server.resume_queue(queue()));
    record!("pause_scheduler", server.pause_scheduler(scheduler()));
    record!("resume_scheduler", server.resume_scheduler(scheduler()));
    record!(
        "add_job",
        server.add_job(Parameters(AddJobInput {
            connection_id: connection_id.into(),
            queue_ref: queue_ref(),
            name: "send-email".into(),
            data: serde_json::json!({ "to": "a@b.c" }),
            delay_ms: None,
            attempts: None,
            priority: None,
            lifo: None,
        }))
    );
    // read-only first; otherwise the empty text is refused
    // (a note needs no approval, so it never waits here).
    record!(
        "annotate_cause",
        server.annotate_cause(Parameters(AnnotateCauseInput {
            connection_id: connection_id.into(),
            queue_ref: queue_ref(),
            signature: "connect ECONNREFUSED <addr>".into(),
            text: "".into(),
            steps: vec![],
            jobs_read: vec![],
        }))
    );

    record!(
        "drain_queue",
        server.drain_queue(Parameters(DrainQueueInput {
            connection_id: connection_id.into(),
            queue_ref: queue_ref(),
            include_delayed: None,
        }))
    );
    record!(
        "clean_queue",
        server.clean_queue(Parameters(CleanQueueInput {
            connection_id: connection_id.into(),
            queue_ref: queue_ref(),
            state: "completed".into(),
            older_than_ms: 0,
        }))
    );

    record!(
        "upsert_scheduler",
        server.upsert_scheduler(Parameters(UpsertSchedulerInput {
            connection_id: connection_id.into(),
            queue_ref: queue_ref(),
            scheduler_id: "nightly".into(),
            job_name: "build-report".into(),
            pattern: Some("0 3 * * *".into()),
            tz: Some("Europe/Paris".into()),
            every_ms: None,
            data: None,
        }))
    );
    record!("remove_scheduler", server.remove_scheduler(scheduler()));

    // The matrix must cover every declared write tool — adding one to the
    // registry without adding it here fails (new write tools must
    // include this test).
    let declared: Vec<&str> = registry::mcp_tool_names()
        .into_iter()
        .filter(|t| registry::mcp_tool_kind(t) == Some(ToolKind::Write))
        .collect();
    let covered: Vec<&str> = out.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(covered, declared, "write-tool matrix is out of date");
    out
}

// ── the registry is the contract ────────────────────────────────────────────

#[test]
fn every_exposed_tool_is_classified_and_nothing_else_is() {
    let mut live = RanchMcpServer::live_tool_names();
    live.sort();
    let mut declared: Vec<String> = registry::mcp_tool_names()
        .into_iter()
        .map(str::to_string)
        .collect();
    declared.sort();
    assert_eq!(
        live, declared,
        "the #[tool] methods of RanchMcpServer and registry::MCP_TOOLS drifted — every exposed tool needs a Read/Write classification"
    );
    for tool in &live {
        assert!(
            registry::mcp_tool_kind(tool).is_some(),
            "{tool} has no kind"
        );
    }
}

#[test]
fn write_tools_are_the_expected_set() {
    let writes: Vec<&str> = registry::mcp_tool_names()
        .into_iter()
        .filter(|t| registry::mcp_tool_kind(t) == Some(ToolKind::Write))
        .collect();
    assert_eq!(
        writes,
        vec![
            "retry_job",
            "remove_job",
            "promote_job",
            "bulk_retry_jobs",
            "bulk_remove_jobs",
            "bulk_promote_jobs",
            "pause_queue",
            "resume_queue",
            "pause_scheduler",
            "resume_scheduler",
            "add_job",
            "annotate_cause",
            "drain_queue",
            "clean_queue",
            "upsert_scheduler",
            "remove_scheduler",
        ]
    );
}

/// Nothing that loosens a safety control may ever be an MCP tool.
#[test]
fn safety_commands_are_never_mcp_tools() {
    let live = RanchMcpServer::live_tool_names();
    let ui = registry::ui_command_names();
    for forbidden in registry::mcp_forbidden_commands() {
        assert!(
            !live.iter().any(|t| t == forbidden),
            "`{forbidden}` must never be exposed over MCP"
        );
        assert!(
            registry::mcp_tool_kind(forbidden).is_none(),
            "`{forbidden}` must not be declared in MCP_TOOLS"
        );
        assert!(
            ui.contains(&forbidden),
            "`{forbidden}` is on the deny-list but is not a UI command — stale entry?"
        );
    }
}

// ── Every write tool is rejected on an MCP read-only connection ─────────────

#[tokio::test]
async fn explicit_readonly_blocks_every_write_tool_before_the_actor() {
    let (_dir, db, server, kc) = fresh().await;
    let conn = add_connection_inner(add_args("redis", "Local", "env-local"), db.pool(), &kc)
        .await
        .unwrap();
    set_connection_mcp_readonly_inner(&conn.id, Some(true), db.pool())
        .await
        .unwrap();

    for (tool, kind) in write_tool_error_kinds(&server, &conn.id).await {
        assert_eq!(kind, "blockedReadonly", "{tool} was not blocked");
    }

    // The message tells the human what to change.
    let err = server.retry_job(job_input(&conn.id)).await.unwrap_err();
    assert!(err.message.contains("read-only for agents"));
    assert!(err.message.contains("Agents › Setup & safety"));
    assert!(err.message.contains("retry_job"));
}

#[tokio::test]
async fn reads_are_never_restricted() {
    let (_dir, db, server, kc) = fresh().await;
    let conn = add_connection_inner(add_args("redis", "Local", "env-local"), db.pool(), &kc)
        .await
        .unwrap();
    set_connection_mcp_readonly_inner(&conn.id, Some(true), db.pool())
        .await
        .unwrap();

    let listed = server.list_connections().await.unwrap();
    // …and the agent can see the flag up-front.
    assert!(listed.contains("\"mcpReadonly\":true"), "got: {listed}");
    server.list_environments().await.unwrap();
}

#[tokio::test]
async fn unset_follows_the_global_default_and_an_explicit_false_overrides_it() {
    let (_dir, db, server, kc) = fresh().await;
    let conn = add_connection_inner(add_args("redis", "Local", "env-local"), db.pool(), &kc)
        .await
        .unwrap();
    assert_eq!(conn.mcp_readonly, None);

    // Approval is the next step of the gate; switch it off so "let through" means
    // "reached the (unreachable) Redis".
    let off = McpSafetySettings {
        readonly_default: false,
        approval_mode: ApprovalMode::Off,
        approval_timeout_seconds: 120,
        approval_always_on_top: true,
        approval_notify_sound: true,
    };
    set_mcp_safety_settings_inner(&off, db.pool())
        .await
        .unwrap();

    // Default off → let through (then fails on the unreachable Redis, NOT on the gate).
    assert!(
        !get_mcp_safety_settings_inner(db.pool())
            .await
            .unwrap()
            .readonly_default
    );
    for (tool, kind) in write_tool_error_kinds(&server, &conn.id).await {
        assert_ne!(
            kind, "blockedReadonly",
            "{tool} was blocked with the default off"
        );
    }

    // Default on → blocked.
    set_mcp_safety_settings_inner(
        &McpSafetySettings {
            readonly_default: true,
            ..off.clone()
        },
        db.pool(),
    )
    .await
    .unwrap();
    assert!(effective_mcp_readonly(db.pool(), &conn.id).await.unwrap().0);
    for (tool, kind) in write_tool_error_kinds(&server, &conn.id).await {
        assert_eq!(kind, "blockedReadonly", "{tool}");
    }

    // Explicit `false` wins over the default; `None` goes back to following it.
    set_connection_mcp_readonly_inner(&conn.id, Some(false), db.pool())
        .await
        .unwrap();
    assert!(!effective_mcp_readonly(db.pool(), &conn.id).await.unwrap().0);
    set_connection_mcp_readonly_inner(&conn.id, None, db.pool())
        .await
        .unwrap();
    assert!(effective_mcp_readonly(db.pool(), &conn.id).await.unwrap().0);
}

// ── production defaults ─────────────────────────────────────────────────────

#[tokio::test]
async fn production_connections_are_mcp_readonly_by_default() {
    let (_dir, db, server, kc) = fresh().await;
    let prod = add_connection_inner(
        add_args("prod", "Production", "env-production"),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    assert_eq!(prod.mcp_readonly, Some(true));
    for (tool, kind) in write_tool_error_kinds(&server, &prod.id).await {
        assert_eq!(kind, "blockedReadonly", "{tool}");
    }
}

#[tokio::test]
async fn entering_production_by_move_or_recolor_makes_a_connection_mcp_readonly() {
    let (_dir, db, _server, kc) = fresh().await;
    let moved = add_connection_inner(add_args("a", "Local", "env-local"), db.pool(), &kc)
        .await
        .unwrap();
    let recolored = add_connection_inner(add_args("b", "Staging", "env-staging"), db.pool(), &kc)
        .await
        .unwrap();
    // The user had explicitly allowed agent writes on it…
    set_connection_mcp_readonly_inner(&recolored.id, Some(false), db.pool())
        .await
        .unwrap();

    let prod_env =
        create_environment_inner(&moved.group_id, "Production", "env-production", db.pool())
            .await
            .unwrap();
    move_connection_inner(&moved.id, &prod_env, db.pool())
        .await
        .unwrap();
    // …turning its environment Production overrides that: the safe side wins.
    change_environment_color_inner(&recolored.environment_id, "env-production", db.pool())
        .await
        .unwrap();

    let rows = list_connections_inner(db.pool()).await.unwrap();
    for id in [&moved.id, &recolored.id] {
        let row = rows.iter().find(|c| &c.id == id).unwrap();
        assert_eq!(row.mcp_readonly, Some(true), "{}", row.name);
    }
}

#[tokio::test]
async fn unknown_connection_is_reported_as_such_not_as_readonly() {
    let (_dir, _db, server, _kc) = fresh().await;
    let err = server.retry_job(job_input("nope")).await.unwrap_err();
    assert_eq!(kind_of(&err), "connectionNotFound");
    match set_connection_mcp_readonly_inner("nope", Some(true), _db.pool()).await {
        Err(ranch_lib::core::errors::RanchError::ConnectionNotFound { .. }) => {}
        other => panic!("expected ConnectionNotFound, got {other:?}"),
    }
}

/// Migration 0005 backfill: a Production connection that predates the
/// MCP safety gate becomes MCP read-only; the others are left to the default.
#[tokio::test]
async fn migration_backfills_existing_production_connections() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn KeychainAdapter> = Arc::new(InMemoryKeychain::new());
    let prod = add_connection_inner(
        add_args("p", "Production", "env-production"),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    let local = add_connection_inner(add_args("l", "Local", "env-local"), db.pool(), &kc)
        .await
        .unwrap();

    // Simulate the pre-migration state, then re-run the migration's backfill statement.
    sqlx::query("UPDATE connections SET mcp_readonly = NULL")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query(
        "UPDATE connections SET mcp_readonly = 1 WHERE environment_id IN (SELECT id FROM environments WHERE color_token = 'env-production')",
    )
    .execute(db.pool())
    .await
    .unwrap();

    let rows = list_connections_inner(db.pool()).await.unwrap();
    assert_eq!(
        rows.iter().find(|c| c.id == prod.id).unwrap().mcp_readonly,
        Some(true)
    );
    assert_eq!(
        rows.iter().find(|c| c.id == local.id).unwrap().mcp_readonly,
        None
    );
}

// ── Query arguments ─────────────────────────────────────────────────────────

#[tokio::test]
async fn bulk_tools_validate_their_input_before_anything_else() {
    use ranch_lib::core::mcp::server::{BulkJobsInput, MCP_BULK_MAX_JOBS};
    let (_dir, db, server, kc) = fresh().await;
    let conn = add_connection_inner(add_args("redis", "Local", "env-local"), db.pool(), &kc)
        .await
        .unwrap();
    let input = |ids: Vec<String>| {
        Parameters(BulkJobsInput {
            connection_id: conn.id.clone(),
            queue_ref: McpQueueRef {
                prefix: "bull".into(),
                name: "emails".into(),
            },
            job_ids: ids,
        })
    };

    let err = server.bulk_remove_jobs(input(vec![])).await.unwrap_err();
    assert_eq!(kind_of(&err), "invalidInput");
    assert!(err.message.contains("cannot be empty"));

    // bulk_retry_jobs: ids or the all-failed selector, never both.
    use ranch_lib::core::mcp::server::{BulkRetryInput, CleanQueueInput};
    let retry = |ids: Vec<String>, all_failed: bool| {
        Parameters(BulkRetryInput {
            connection_id: conn.id.clone(),
            queue_ref: McpQueueRef {
                prefix: "bull".into(),
                name: "emails".into(),
            },
            job_ids: ids,
            all_failed,
        })
    };
    let err = server
        .bulk_retry_jobs(retry(vec![], false))
        .await
        .unwrap_err();
    assert!(err.message.contains("cannot be empty"));
    let err = server
        .bulk_retry_jobs(retry(vec!["1".into()], true))
        .await
        .unwrap_err();
    assert_eq!(kind_of(&err), "invalidInput");
    assert!(err.message.contains("not both"));

    let err = server
        .clean_queue(Parameters(CleanQueueInput {
            connection_id: conn.id.clone(),
            queue_ref: McpQueueRef {
                prefix: "bull".into(),
                name: "emails".into(),
            },
            state: "active".into(),
            older_than_ms: 0,
        }))
        .await
        .unwrap_err();
    assert_eq!(kind_of(&err), "invalidInput");
    assert!(err.message.contains("completed or failed"));

    // A scheduler is validated before anyone is asked to approve.
    use ranch_lib::core::mcp::server::UpsertSchedulerInput;
    let scheduler = |pattern: Option<&str>, every_ms: Option<u64>, tz: Option<&str>| {
        Parameters(UpsertSchedulerInput {
            connection_id: conn.id.clone(),
            queue_ref: McpQueueRef {
                prefix: "bull".into(),
                name: "emails".into(),
            },
            scheduler_id: "nightly".into(),
            job_name: "build-report".into(),
            pattern: pattern.map(Into::into),
            tz: tz.map(Into::into),
            every_ms,
            data: None,
        })
    };
    for (input, expected) in [
        (scheduler(None, None, None), "cron pattern or everyMs"),
        (scheduler(Some("0 3 * * *"), Some(5_000), None), "not both"),
        (scheduler(Some("0 3 L * *"), None, None), "not supported"),
        (
            scheduler(Some("0 3 * * *"), None, Some("Mars/Olympus")),
            "Unknown timezone",
        ),
        (scheduler(None, Some(10), None), "at least 1000 ms"),
    ] {
        let err = server.upsert_scheduler(input).await.unwrap_err();
        assert_eq!(kind_of(&err), "invalidInput", "{expected}");
        assert!(
            err.message.contains(expected),
            "{}: {expected}",
            err.message
        );
    }

    let too_many = (0..=MCP_BULK_MAX_JOBS).map(|n| n.to_string()).collect();
    let err = server.bulk_remove_jobs(input(too_many)).await.unwrap_err();
    assert_eq!(kind_of(&err), "invalidInput");
    assert!(err.message.contains("Split the request"));
}

/// Mode `all`: reads that target a connection are held too; the discovery
/// tools (no connection, no queue data) stay free.
#[tokio::test]
async fn mode_all_holds_connection_reads_but_not_discovery() {
    use ranch_lib::core::mcp::server::{ConnectionIdInput, QueueInput};
    let (_dir, db, server, kc) = fresh().await;
    let conn = add_connection_inner(add_args("redis", "Local", "env-local"), db.pool(), &kc)
        .await
        .unwrap();
    set_mcp_safety_settings_inner(
        &McpSafetySettings {
            readonly_default: false,
            approval_mode: ApprovalMode::All,
            approval_timeout_seconds: 120,
            approval_always_on_top: true,
            approval_notify_sound: true,
        },
        db.pool(),
    )
    .await
    .unwrap();

    // No GUI heartbeat in this test → a held call fails fast with hostUnavailable.
    let err = server
        .list_queues(Parameters(ConnectionIdInput {
            connection_id: conn.id.clone(),
        }))
        .await
        .unwrap_err();
    assert_eq!(kind_of(&err), "hostUnavailable");
    let err = server
        .list_schedulers(Parameters(QueueInput {
            connection_id: conn.id.clone(),
            queue_ref: McpQueueRef {
                prefix: "bull".into(),
                name: "emails".into(),
            },
        }))
        .await
        .unwrap_err();
    assert_eq!(kind_of(&err), "hostUnavailable");

    server.list_connections().await.unwrap();
    server.list_environments().await.unwrap();
}
