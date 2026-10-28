//! MCP activity log.

mod common;
use common::{kv, repo};

use std::sync::Arc;
use std::time::{Duration, Instant};

use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::settings::set_kv_setting_inner;
use ranch_lib::core::mcp::activity::{
    self, compact_arguments, ActivityFilter, ActivityStatus, NewActivity, ACTIVITY_ENABLED_KEY,
    ACTIVITY_MAX_ENTRIES_KEY, SESSION_GAP_MINUTES_KEY,
};
use ranch_lib::core::mcp::gate::ToolKind;
use ranch_lib::shells::mcp::RanchMcpServer;
use rmcp::model::CallToolResult;
use rmcp::ErrorData;
use tempfile::tempdir;

async fn fresh() -> (tempfile::TempDir, Db, RanchMcpServer, String) {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "queues".into(),
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
            environment: EnvironmentSelector::named("Staging", "env-staging"),
        },
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let server = RanchMcpServer::new(common::mcp_state(
        db.pool(),
        Arc::new(common::backends(db.pool(), &kc, &ConnectionRegistry::new())),
    ));
    server
        .set_client_info_for_test("claude-desktop", "1.0")
        .await;
    (dir, db, server, conn.id)
}

fn ok() -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![]))
}

fn err(kind: &str, message: &str) -> Result<CallToolResult, ErrorData> {
    Err(ErrorData::internal_error(
        message.to_string(),
        Some(serde_json::json!({ "kind": kind })),
    ))
}

fn new_event(session: &str, tool: &str, kind: ToolKind, status: ActivityStatus) -> NewActivity {
    NewActivity {
        session_id: session.into(),
        client_hint: Some("cursor".into()),
        tool: tool.into(),
        kind,
        connection_id: None,
        arguments: None,
        status,
        duration: Duration::from_millis(12),
        error: None,
        approval_id: None,
    }
}

#[tokio::test]
async fn every_call_is_recorded_reads_included_with_the_right_status() {
    let (_dir, db, server, conn_id) = fresh().await;
    let args = |tool_args: serde_json::Value| Some(tool_args);
    let t0 = Instant::now();

    server
        .record_tool_call("list_connections", None, &ok(), t0)
        .await;
    server
        .record_tool_call(
            "list_jobs",
            args(serde_json::json!({ "connectionId": conn_id, "state": "failed" })),
            &ok(),
            t0,
        )
        .await;
    for (kind, status) in [
        ("blockedReadonly", ActivityStatus::BlockedReadonly),
        ("approvalDenied", ActivityStatus::Denied),
        ("approvalTimeout", ActivityStatus::Timeout),
        ("hostUnavailable", ActivityStatus::HostUnavailable),
        ("jobNotFound", ActivityStatus::Error),
    ] {
        server
            .record_tool_call(
                "remove_job",
                args(serde_json::json!({ "connectionId": conn_id, "jobId": "42" })),
                &err(kind, "nope"),
                t0,
            )
            .await;
        let last = &activity::list(db.pool(), &ActivityFilter::default(), 1, 0)
            .await
            .unwrap()[0];
        assert_eq!(last.status, status, "{kind}");
        assert_eq!(last.error.as_deref(), Some("nope"));
    }

    let all = activity::list(db.pool(), &ActivityFilter::default(), 50, 0)
        .await
        .unwrap();
    assert_eq!(all.len(), 7);
    // Newest first; the oldest is the connection-less read.
    let first = all.last().unwrap();
    assert_eq!(first.tool, "list_connections");
    assert_eq!(first.kind, "read");
    assert_eq!(first.status, ActivityStatus::Success);
    assert_eq!(first.connection_id, None);
    assert_eq!(first.client_hint.as_deref(), Some("claude-desktop"));

    let read = &all[all.len() - 2];
    assert_eq!(read.connection_name.as_deref(), Some("queues"));
    assert!(read.target_json.as_deref().unwrap().contains("failed"));

    assert_eq!(all[0].kind, "write");
    // One process, no long silence → one session.
    assert!(all.iter().all(|e| e.session_id == all[0].session_id));
}

#[tokio::test]
async fn the_approval_a_call_went_through_is_linked() {
    use ranch_lib::core::mcp::approval::{self, NewApproval};
    let (_dir, db, server, conn_id) = fresh().await;
    let started = Instant::now();
    let approval_id = approval::enqueue(
        db.pool(),
        &kv(db.pool()),
        &NewApproval {
            client_hint: None,
            connection_id: conn_id.clone(),
            tool: "remove_job".into(),
            kind: ToolKind::Write,
            arguments: serde_json::json!({}),
            target_preview: None,
        },
    )
    .await
    .unwrap();

    server
        .record_tool_call(
            "remove_job",
            Some(serde_json::json!({ "connectionId": conn_id })),
            &err("approvalDenied", "Denied by the user"),
            started,
        )
        .await;
    // A different tool on the same connection is not linked to it.
    server
        .record_tool_call(
            "retry_job",
            Some(serde_json::json!({ "connectionId": conn_id })),
            &ok(),
            started,
        )
        .await;

    let all = activity::list(db.pool(), &ActivityFilter::default(), 10, 0)
        .await
        .unwrap();
    assert_eq!(all[1].approval_id.as_deref(), Some(approval_id.as_str()));
    assert_eq!(all[0].approval_id, None);
}

#[tokio::test]
async fn sessions_rotate_after_a_long_silence() {
    let (_dir, db, server, _conn) = fresh().await;
    let t0 = Instant::now();
    server
        .record_tool_call("list_connections", None, &ok(), t0)
        .await;
    server
        .record_tool_call("list_connections", None, &ok(), t0)
        .await;

    // A 0-minute gap is refused (kept at the default); use the smallest real one
    // and make the session look old instead.
    set_kv_setting_inner(&kv(db.pool()), SESSION_GAP_MINUTES_KEY, "1")
        .await
        .unwrap();
    server.age_session_for_test(Duration::from_secs(61)).await;
    server
        .record_tool_call("list_connections", None, &ok(), t0)
        .await;

    let sessions = activity::list_sessions(db.pool(), 10).await.unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].total, 1, "most recent session first");
    assert_eq!(sessions[1].total, 2);
    assert_eq!(sessions[1].client_hint.as_deref(), Some("claude-desktop"));
}

#[tokio::test]
async fn disabled_writes_nothing_and_the_cap_drops_the_oldest() {
    let (_dir, db, _server, _conn) = fresh().await;

    set_kv_setting_inner(&kv(db.pool()), ACTIVITY_ENABLED_KEY, "false")
        .await
        .unwrap();
    activity::record(
        db.pool(),
        &kv(db.pool()),
        new_event("s1", "list_queues", ToolKind::Read, ActivityStatus::Success),
    )
    .await
    .unwrap();
    assert_eq!(
        activity::count(db.pool(), &ActivityFilter::default())
            .await
            .unwrap(),
        0
    );

    set_kv_setting_inner(&kv(db.pool()), ACTIVITY_ENABLED_KEY, "true")
        .await
        .unwrap();
    set_kv_setting_inner(&kv(db.pool()), ACTIVITY_MAX_ENTRIES_KEY, "3")
        .await
        .unwrap();
    for n in 0..5 {
        activity::record(
            db.pool(),
            &kv(db.pool()),
            new_event(
                "s1",
                &format!("tool_{n}"),
                ToolKind::Read,
                ActivityStatus::Success,
            ),
        )
        .await
        .unwrap();
    }
    let kept = activity::list(db.pool(), &ActivityFilter::default(), 10, 0)
        .await
        .unwrap();
    assert_eq!(
        kept.iter().map(|e| e.tool.as_str()).collect::<Vec<_>>(),
        vec!["tool_4", "tool_3", "tool_2"]
    );
}

#[tokio::test]
async fn filters_sessions_summary_and_clear() {
    let (_dir, db, _server, _conn) = fresh().await;
    for (session, tool, kind, status) in [
        ("s1", "list_jobs", ToolKind::Read, ActivityStatus::Success),
        ("s1", "remove_job", ToolKind::Write, ActivityStatus::Denied),
        ("s2", "retry_job", ToolKind::Write, ActivityStatus::Success),
    ] {
        activity::record(
            db.pool(),
            &kv(db.pool()),
            new_event(session, tool, kind, status),
        )
        .await
        .unwrap();
    }

    let writes = ActivityFilter {
        kind: Some("write".into()),
        ..Default::default()
    };
    assert_eq!(activity::count(db.pool(), &writes).await.unwrap(), 2);
    let denied = ActivityFilter {
        status: Some(ActivityStatus::Denied),
        ..Default::default()
    };
    let rows = activity::list(db.pool(), &denied, 10, 0).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tool, "remove_job");
    let s1 = ActivityFilter {
        session_id: Some("s1".into()),
        ..Default::default()
    };
    assert_eq!(activity::count(db.pool(), &s1).await.unwrap(), 2);

    let sessions = activity::list_sessions(db.pool(), 10).await.unwrap();
    let first = sessions.iter().find(|s| s.session_id == "s1").unwrap();
    assert_eq!((first.total, first.writes, first.not_successful), (2, 1, 1));

    assert_eq!(activity::clear(db.pool()).await.unwrap(), 3);
    assert_eq!(
        activity::count(db.pool(), &ActivityFilter::default())
            .await
            .unwrap(),
        0
    );
}

#[test]
fn long_id_lists_are_compacted() {
    let small = serde_json::json!({ "jobIds": ["1", "2"] });
    assert_eq!(compact_arguments(&small), small.to_string());

    let ids: Vec<String> = (0..1000).map(|n| format!("job-{n:06}")).collect();
    let big = serde_json::json!({ "connectionId": "c1", "jobIds": ids });
    let compacted: serde_json::Value = serde_json::from_str(&compact_arguments(&big)).unwrap();
    assert_eq!(compacted["connectionId"], "c1");
    assert_eq!(compacted["jobIds"]["count"], 1000);
    assert_eq!(compacted["jobIds"]["first"].as_array().unwrap().len(), 20);
}

#[tokio::test]
async fn csv_export_quotes_and_defuses_spreadsheet_formulas() {
    let (_dir, db, _server, _conn) = fresh().await;
    let mut event = new_event("s1", "remove_job", ToolKind::Write, ActivityStatus::Error);
    // Agent- and Redis-controlled text ends up in these cells.
    event.error = Some("=HYPERLINK(\"http://evil\",\"click\")".into());
    event.arguments = Some(serde_json::json!({ "note": "a \"quoted\", value" }));
    activity::record(db.pool(), &kv(db.pool()), event)
        .await
        .unwrap();

    let csv = activity::export(db.pool(), &ActivityFilter::default(), true)
        .await
        .unwrap();
    let mut lines = csv.lines();
    assert!(lines
        .next()
        .unwrap()
        .starts_with("timestamp,session_id,client,tool"));
    let row = lines.next().unwrap();
    assert!(
        row.contains("\"'=HYPERLINK("),
        "formula must be prefixed: {row}"
    );
    assert!(
        row.contains("\"\"click\"\""),
        "quotes must be doubled: {row}"
    );
    // The JSON cell keeps its own backslash escapes, inside doubled CSV quotes.
    assert!(
        row.contains("\"{\"\"note\"\":"),
        "the JSON must stay one quoted cell: {row}"
    );

    let json = activity::export(db.pool(), &ActivityFilter::default(), false)
        .await
        .unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed[0]["tool"], "remove_job");
    assert_eq!(parsed[0]["status"], "error");
}
