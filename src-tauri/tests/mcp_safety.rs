//! MCP safety gate.
//!
//! Step 1 of the gate: a connection that is read-only for agents rejects
//! every MCP **write** tool with `blockedReadonly`, before anything reaches
//! the connection actor (no Redis needed for these tests; that is the
//! point). Read tools are never restricted.

mod common;
use common::{kv, repo};

use std::sync::Arc;

use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::catalogue::{self, Mcp};
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{
    add_connection_inner, list_connections_inner, AddConnectionArgs,
};
use ranch_lib::commands::hierarchy::{
    change_environment_color_inner, create_environment_inner, move_connection_inner,
    EnvironmentSelector, GroupSelector,
};
use ranch_lib::commands::mcp_safety::ApprovalMode;
use ranch_lib::commands::mcp_safety::{
    get_mcp_safety_settings_inner, set_connection_mcp_readonly_inner,
    set_mcp_safety_settings_inner, McpSafetySettings,
};
use ranch_lib::core::mcp::gate::effective_mcp_readonly;
use ranch_lib::shells::mcp::server::{JobActionInput, McpQueueId};
use ranch_lib::shells::mcp::RanchMcpServer;
use rmcp::handler::server::wrapper::Parameters;
use tempfile::tempdir;

async fn fresh() -> (tempfile::TempDir, Db, RanchMcpServer, Arc<dyn SecretStore>) {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let keychain: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let server = RanchMcpServer::new(common::mcp_state(
        db.pool(),
        Arc::new(common::backends(
            db.pool(),
            &keychain,
            &ConnectionRegistry::new(),
        )),
    ));
    (dir, db, server, keychain)
}

fn add_args(name: &str, env_name: &str, color: &str) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.into(),
        // Nothing listens here: a call that gets PAST the gate fails fast on
        // the connection, which is how the tests tell "blocked" from "let through".
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
        group: GroupSelector::named("Lumen"),
        environment: EnvironmentSelector::named(env_name, color),
    }
}

fn job_input(connection_id: &str) -> Parameters<JobActionInput> {
    Parameters(JobActionInput {
        connection_id: connection_id.into(),
        queue_id: McpQueueId("bull:emails".into()),
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
    use ranch_lib::shells::mcp::server::{
        AddJobInput, AnnotateCauseInput, BulkJobsInput, BulkRetryInput, CleanQueueInput,
        DrainQueueInput, QueueInput, SchedulerInput, UpsertSchedulerInput,
    };
    let queue_id = || McpQueueId("bull:emails".into());
    let queue = || {
        Parameters(QueueInput {
            connection_id: connection_id.into(),
            queue_id: queue_id(),
        })
    };
    let scheduler = || {
        Parameters(SchedulerInput {
            connection_id: connection_id.into(),
            queue_id: queue_id(),
            scheduler_key: "nightly".into(),
        })
    };
    let bulk = || {
        Parameters(BulkJobsInput {
            connection_id: connection_id.into(),
            queue_id: queue_id(),
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
            queue_id: queue_id(),
            job_ids: vec!["1".into(), "2".into()],
            all_failed: false,
        }))
    );
    // The all-failed selector is the same tool: same gate, same answer.
    let all_failed = server
        .bulk_retry_jobs(Parameters(BulkRetryInput {
            connection_id: connection_id.into(),
            queue_id: queue_id(),
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
            queue_id: queue_id(),
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
            queue_id: queue_id(),
            queue: None,
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
            queue_id: queue_id(),
            include_delayed: None,
        }))
    );
    record!(
        "clean_queue",
        server.clean_queue(Parameters(CleanQueueInput {
            connection_id: connection_id.into(),
            queue_id: queue_id(),
            state: "completed".into(),
            older_than_ms: 0,
        }))
    );

    record!(
        "upsert_scheduler",
        server.upsert_scheduler(Parameters(UpsertSchedulerInput {
            connection_id: connection_id.into(),
            queue_id: queue_id(),
            scheduler_id: "nightly".into(),
            job_name: "build-report".into(),
            pattern: Some("0 3 * * *".into()),
            tz: Some("Europe/Paris".into()),
            every_ms: None,
            data: None,
        }))
    );
    record!("remove_scheduler", server.remove_scheduler(scheduler()));

    // The matrix must cover every write tool of the catalogue: one added
    // there without a call here fails.
    let mut declared = write_tools();
    declared.sort();
    let mut covered: Vec<&str> = out.iter().map(|(n, _)| n.as_str()).collect();
    covered.sort();
    assert_eq!(covered, declared, "write-tool matrix is out of date");
    out
}

/// The catalogue's MCP write tools, in its order.
fn write_tools() -> Vec<&'static str> {
    catalogue::tools()
        .filter(|(_, op)| op.kind.is_write())
        .map(|(name, _)| name)
        .collect()
}

// ── the catalogue is the contract ───────────────────────────────────────────

#[test]
fn every_exposed_tool_is_classified_and_nothing_else_is() {
    let mut live = RanchMcpServer::live_tool_names();
    live.sort();
    let mut declared: Vec<String> = catalogue::tools()
        .map(|(name, _)| name.to_string())
        .collect();
    declared.sort();
    assert_eq!(
        live, declared,
        "the #[tool] methods of RanchMcpServer and the catalogue's tools drifted; every exposed tool needs an operation, read or write"
    );
}

#[test]
fn write_tools_are_the_expected_set() {
    let mut writes = write_tools();
    writes.sort();
    assert_eq!(
        writes,
        vec![
            "add_job",
            "annotate_cause",
            "bulk_promote_jobs",
            "bulk_remove_jobs",
            "bulk_retry_jobs",
            "clean_queue",
            "drain_queue",
            "pause_queue",
            "pause_scheduler",
            "promote_job",
            "remove_job",
            "remove_scheduler",
            "resume_queue",
            "resume_scheduler",
            "retry_job",
            "upsert_scheduler",
        ]
    );
}

/// Nothing that loosens a safety control may ever be an MCP tool.
#[test]
fn safety_commands_are_never_mcp_tools() {
    let live = RanchMcpServer::live_tool_names();
    let forbidden: Vec<&str> = catalogue::OPERATIONS
        .iter()
        .filter(|op| matches!(op.mcp, Mcp::Forbidden(_)))
        .map(|op| op.name)
        .collect();
    for name in &forbidden {
        assert!(
            !live.iter().any(|t| t == name),
            "`{name}` must never be exposed over MCP"
        );
    }
    // The deny-list, pinned: shortening it is a conscious change.
    assert_eq!(
        forbidden,
        [
            "remove_connection",
            "delete_group",
            "set_kv_setting",
            "dismiss_cause_note",
            "unlock_connection",
            "lock_connection",
            "set_mcp_safety_settings",
            "set_ai_features",
            "set_connection_mcp_readonly",
            "list_pending_approvals",
            "decide_pending_approval",
            "get_approval_record",
            "list_approval_history",
            "list_mcp_activity",
            "list_mcp_activity_sessions",
            "export_mcp_activity",
            "clear_mcp_activity",
            "install_mcp_for_client",
            "launch_claude_code",
            "open_agent_app",
        ]
    );
}

// ── Every write tool is rejected on an MCP read-only connection ─────────────

#[tokio::test]
async fn explicit_readonly_blocks_every_write_tool_before_the_actor() {
    let (_dir, db, server, kc) = fresh().await;
    let conn = add_connection_inner(
        add_args("redis", "Local", "env-local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    set_connection_mcp_readonly_inner(&conn.id, Some(true), &repo(db.pool()))
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
    let conn = add_connection_inner(
        add_args("redis", "Local", "env-local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    set_connection_mcp_readonly_inner(&conn.id, Some(true), &repo(db.pool()))
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
    let conn = add_connection_inner(
        add_args("redis", "Local", "env-local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
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
    set_mcp_safety_settings_inner(&off, &kv(db.pool()))
        .await
        .unwrap();

    // Default off → let through (then fails on the unreachable Redis, NOT on the gate).
    assert!(
        !get_mcp_safety_settings_inner(&kv(db.pool()))
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
        &kv(db.pool()),
    )
    .await
    .unwrap();
    assert!(
        effective_mcp_readonly(&kv(db.pool()), &repo(db.pool()), &conn.id)
            .await
            .unwrap()
            .0
    );
    for (tool, kind) in write_tool_error_kinds(&server, &conn.id).await {
        assert_eq!(kind, "blockedReadonly", "{tool}");
    }

    // Explicit `false` wins over the default; `None` goes back to following it.
    set_connection_mcp_readonly_inner(&conn.id, Some(false), &repo(db.pool()))
        .await
        .unwrap();
    assert!(
        !effective_mcp_readonly(&kv(db.pool()), &repo(db.pool()), &conn.id)
            .await
            .unwrap()
            .0
    );
    set_connection_mcp_readonly_inner(&conn.id, None, &repo(db.pool()))
        .await
        .unwrap();
    assert!(
        effective_mcp_readonly(&kv(db.pool()), &repo(db.pool()), &conn.id)
            .await
            .unwrap()
            .0
    );
}

// ── production defaults ─────────────────────────────────────────────────────

#[tokio::test]
async fn production_connections_are_mcp_readonly_by_default() {
    let (_dir, db, server, kc) = fresh().await;
    let prod = add_connection_inner(
        add_args("prod", "Production", "env-production"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
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
    let moved = add_connection_inner(
        add_args("a", "Local", "env-local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let recolored = add_connection_inner(
        add_args("b", "Staging", "env-staging"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    // The user had explicitly allowed agent writes on it…
    set_connection_mcp_readonly_inner(&recolored.id, Some(false), &repo(db.pool()))
        .await
        .unwrap();

    let prod_env = create_environment_inner(
        &moved.group_id,
        "Production",
        "env-production",
        &repo(db.pool()),
    )
    .await
    .unwrap();
    move_connection_inner(&moved.id, &prod_env, &repo(db.pool()))
        .await
        .unwrap();
    // …turning its environment Production overrides that: the safe side wins.
    change_environment_color_inner(
        &recolored.environment_id,
        "env-production",
        &repo(db.pool()),
    )
    .await
    .unwrap();

    let rows = list_connections_inner(&repo(db.pool())).await.unwrap();
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
    match set_connection_mcp_readonly_inner("nope", Some(true), &repo(_db.pool())).await {
        Err(ranch_lib::domain::errors::RanchError::ConnectionNotFound { .. }) => {}
        other => panic!("expected ConnectionNotFound, got {other:?}"),
    }
}

/// Migration 0005 backfill: a Production connection that predates the
/// MCP safety gate becomes MCP read-only; the others are left to the default.
#[tokio::test]
async fn migration_backfills_existing_production_connections() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let prod = add_connection_inner(
        add_args("p", "Production", "env-production"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let local = add_connection_inner(
        add_args("l", "Local", "env-local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
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

    let rows = list_connections_inner(&repo(db.pool())).await.unwrap();
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
    use ranch_lib::shells::mcp::server::{BulkJobsInput, MCP_BULK_MAX_JOBS};
    let (_dir, db, server, kc) = fresh().await;
    let conn = add_connection_inner(
        add_args("redis", "Local", "env-local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let input = |ids: Vec<String>| {
        Parameters(BulkJobsInput {
            connection_id: conn.id.clone(),
            queue_id: McpQueueId("bull:emails".into()),
            job_ids: ids,
        })
    };

    let err = server.bulk_remove_jobs(input(vec![])).await.unwrap_err();
    assert_eq!(kind_of(&err), "invalidInput");
    assert!(err.message.contains("cannot be empty"));

    // bulk_retry_jobs: ids or the all-failed selector, never both.
    use ranch_lib::shells::mcp::server::{BulkRetryInput, CleanQueueInput};
    let retry = |ids: Vec<String>, all_failed: bool| {
        Parameters(BulkRetryInput {
            connection_id: conn.id.clone(),
            queue_id: McpQueueId("bull:emails".into()),
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
            queue_id: McpQueueId("bull:emails".into()),
            state: "active".into(),
            older_than_ms: 0,
        }))
        .await
        .unwrap_err();
    assert_eq!(kind_of(&err), "invalidInput");
    assert!(err.message.contains("completed or failed"));

    // A scheduler is validated before anyone is asked to approve.
    use ranch_lib::shells::mcp::server::UpsertSchedulerInput;
    let scheduler = |pattern: Option<&str>, every_ms: Option<u64>, tz: Option<&str>| {
        Parameters(UpsertSchedulerInput {
            connection_id: conn.id.clone(),
            queue_id: McpQueueId("bull:emails".into()),
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
    use ranch_lib::shells::mcp::server::{ConnectionIdInput, QueueInput};
    let (_dir, db, server, kc) = fresh().await;
    let conn = add_connection_inner(
        add_args("redis", "Local", "env-local"),
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
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
        &kv(db.pool()),
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
            queue_id: McpQueueId("bull:emails".into()),
        }))
        .await
        .unwrap_err();
    assert_eq!(kind_of(&err), "hostUnavailable");

    server.list_connections().await.unwrap();
    server.list_environments().await.unwrap();
}
