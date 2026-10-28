//! AI features off: the real `ranch-mcp`
//! binary, driven over stdio like an MCP client, refuses every tool it
//! advertises with `aiDisabled`; and turning AI off denies the requests that
//! were waiting for a human.

mod common;
use common::{kv, repo, unsealed};

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::mcp_safety::set_ai_features_inner;
use ranch_lib::core::mcp::approval::{self, NewApproval};
use ranch_lib::core::mcp::gate::{ai_enabled, ToolKind};
use ranch_lib::shells::mcp::RanchMcpServer;
use serde_json::{json, Value};
use tempfile::tempdir;

/// Where `ranch-mcp` looks for its database under a given home.
fn app_data_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/dev.ranch.app")
    } else {
        home.join("dev.ranch.app")
    }
}

fn send(stdin: &mut impl Write, message: Value) {
    writeln!(stdin, "{message}").unwrap();
    stdin.flush().unwrap();
}

/// The next JSON-RPC response carrying `id` (notifications and logs skipped).
fn response(lines: &mut impl Iterator<Item = std::io::Result<String>>, id: u64) -> Value {
    for line in lines.by_ref() {
        let line = line.unwrap();
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("id") == Some(&json!(id)) {
            return value;
        }
    }
    panic!("ranch-mcp closed stdout before answering request {id}");
}

#[tokio::test]
async fn ranch_mcp_refuses_every_tool_while_ai_features_are_off() {
    let home = tempdir().unwrap();
    let data = app_data_dir(home.path());
    std::fs::create_dir_all(&data).unwrap();
    let db = Db::init(&data).await.unwrap();
    assert!(
        ai_enabled(&kv(db.pool())).await,
        "on by default for existing installs"
    );
    assert!(!set_ai_features_inner(db.pool(), &kv(db.pool()), false)
        .await
        .unwrap());
    db.pool().close().await;

    let mut child = Command::new(env!("CARGO_BIN_EXE_ranch-mcp"))
        .env("RANCH_MCP_DATA_DIR", &data)
        .env("RUST_LOG", "error")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("ranch-mcp starts");
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "ai-off-test", "version": "0" }
            }
        }),
    );
    assert!(response(&mut lines, 1).get("result").is_some());
    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    );

    let tools = RanchMcpServer::live_tool_names();
    assert!(!tools.is_empty());
    for (i, tool) in tools.iter().enumerate() {
        let id = 100 + i as u64;
        send(
            &mut stdin,
            json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": { "name": tool, "arguments": {} }
            }),
        );
        let reply = response(&mut lines, id);
        let kind = reply.pointer("/error/data/kind").and_then(Value::as_str);
        assert_eq!(kind, Some("aiDisabled"), "{tool} answered {reply}");
        let message = reply.pointer("/error/message").and_then(Value::as_str);
        assert!(
            message.is_some_and(|m| m.contains("AI features are turned off")),
            "{tool}: {reply}"
        );
    }

    drop(stdin);
    let _ = child.wait();
}

#[tokio::test]
async fn turning_ai_off_denies_the_requests_waiting_for_a_human() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let keychain: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let connection = add_connection_inner(
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
            group: GroupSelector::named("Default"),
            environment: EnvironmentSelector::for_color("env-local"),
        },
        &repo(db.pool()),
        &keychain,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let id = approval::enqueue(
        db.pool(),
        &kv(db.pool()),
        &NewApproval {
            client_hint: Some("claude-desktop".into()),
            connection_id: connection.id,
            tool: "retry_job".into(),
            kind: ToolKind::Write,
            arguments: json!({}),
            target_preview: None,
        },
    )
    .await
    .unwrap();

    set_ai_features_inner(db.pool(), &kv(db.pool()), false)
        .await
        .unwrap();

    assert!(approval::list_pending(db.pool(), &kv(db.pool()))
        .await
        .unwrap()
        .is_empty());
    let record = approval::record(db.pool(), &id).await.unwrap().unwrap();
    assert_eq!(record.status, "denied");
    assert_eq!(
        record.reason.as_deref(),
        Some("AI features were turned off in Ranch")
    );

    // On again: nothing else changes, agents may call again.
    assert!(set_ai_features_inner(db.pool(), &kv(db.pool()), true)
        .await
        .unwrap());
}

#[tokio::test]
async fn the_history_lists_decided_requests_newest_first_with_their_connection() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let keychain: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let connection = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "prod".into(),
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
            environment: EnvironmentSelector::for_color("env-local"),
        },
        &repo(db.pool()),
        &keychain,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let ask = |tool: &str| NewApproval {
        client_hint: Some("cursor".into()),
        connection_id: connection.id.clone(),
        tool: tool.into(),
        kind: ToolKind::Write,
        arguments: json!({}),
        target_preview: None,
    };
    let first = approval::enqueue(db.pool(), &kv(db.pool()), &ask("retry_job"))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let second = approval::enqueue(db.pool(), &kv(db.pool()), &ask("remove_job"))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let _waiting = approval::enqueue(db.pool(), &kv(db.pool()), &ask("promote_job"))
        .await
        .unwrap();
    approval::decide(
        db.pool(),
        &unsealed(),
        &first,
        approval::ApprovalDecision::Approve,
        None,
    )
    .await
    .unwrap();
    approval::decide(
        db.pool(),
        &unsealed(),
        &second,
        approval::ApprovalDecision::Deny,
        Some("not now"),
    )
    .await
    .unwrap();

    let history = approval::history(db.pool(), 10).await.unwrap();
    let seen: Vec<(&str, &str)> = history
        .iter()
        .map(|h| (h.tool.as_str(), h.status.as_str()))
        .collect();
    assert_eq!(seen, [("remove_job", "denied"), ("retry_job", "approved")]);
    assert_eq!(history[0].reason.as_deref(), Some("not now"));
    assert_eq!(history[0].connection_name.as_deref(), Some("prod"));
}
