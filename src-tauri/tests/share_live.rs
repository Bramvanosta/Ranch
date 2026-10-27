//! What agents may see of job data, end to end: the real
//! `ranch-mcp`, driven over stdio, answers every read tool on jobs whose
//! payload, result, progress and scheduler template hold nested sensitive
//! fields; with the default policy (mask) none of their values leaks, and
//! with "names, states and errors only" no payload is sent at all.
//!
//! Needs Ranch's own Redis: `RANCH_LOCAL_REDIS=1` and
//! `docker compose --profile redis up -d` (port 6390).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;

use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::mcp_safety::{
    get_mcp_safety_settings_inner, set_mcp_safety_settings_inner,
};
use ranch_lib::commands::settings::set_kv_setting_inner;
use ranch_lib::core::db::Db;
use ranch_lib::core::keychain::{InMemoryKeychain, KeychainAdapter};
use ranch_lib::core::mcp::approval::ApprovalMode;
use ranch_lib::core::mcp::share::{AI_SHARE_MODE_KEY, MASK};
use serde_json::{json, Value};

const PREFIX: &str = "bull";

/// The values that must never reach an agent in mask mode.
const SECRETS: &[&str] = &[
    "ann@example.com",
    "+33612345678",
    "tok_live_123",
    "hunter2",
    "FR7630006000011234567890189",
    "4111111111111111",
    "sk_template_999",
];

fn skip() -> bool {
    if std::env::var("RANCH_LOCAL_REDIS").is_err() {
        eprintln!("skipping: set RANCH_LOCAL_REDIS=1 + `docker compose --profile redis up -d`");
        return true;
    }
    false
}

fn app_data_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/dev.ranch.app")
    } else {
        home.join("dev.ranch.app")
    }
}

struct Mcp {
    child: Child,
    stdin: ChildStdin,
    lines: std::io::Lines<BufReader<std::process::ChildStdout>>,
    next_id: u64,
}

impl Mcp {
    fn start(home: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ranch-mcp"))
            .env("RANCH_MCP_DATA_DIR", app_data_dir(home))
            .env("RUST_LOG", "error")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("ranch-mcp starts");
        let stdin = child.stdin.take().unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut mcp = Self {
            child,
            stdin,
            lines,
            next_id: 1,
        };
        let init = mcp.request(
            "initialize",
            json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "share-test", "version": "0" }
            }),
        );
        assert!(init.get("result").is_some(), "{init}");
        mcp.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        mcp
    }

    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        for line in self.lines.by_ref() {
            let Ok(value) = serde_json::from_str::<Value>(&line.unwrap()) else {
                continue;
            };
            if value.get("id") == Some(&json!(id)) {
                return value;
            }
        }
        panic!("ranch-mcp closed stdout before answering {method}");
    }

    /// The text of a tool's answer (panics on an error).
    fn call(&mut self, tool: &str, arguments: Value) -> String {
        let reply = self.request(
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        );
        assert!(reply.get("error").is_none(), "{tool}: {reply}");
        reply
            .pointer("/result/content/0/text")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{tool}: no text in {reply}"))
            .to_string()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn cleanup(redis: &mut redis::aio::MultiplexedConnection, queue: &str) {
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg(format!("{PREFIX}:{queue}:*"))
        .query_async(redis)
        .await
        .unwrap();
    if !keys.is_empty() {
        let _: () = redis::cmd("DEL")
            .arg(keys)
            .query_async(redis)
            .await
            .unwrap();
    }
}

/// `wait` → `failed` / `completed`, as a worker would leave it.
async fn finish(
    redis: &mut redis::aio::MultiplexedConnection,
    queue: &str,
    id: &str,
    set: &str,
    fields: &[(&str, String)],
) {
    let base = format!("{PREFIX}:{queue}");
    let _: () = redis::cmd("LREM")
        .arg(format!("{base}:wait"))
        .arg(0)
        .arg(id)
        .query_async(redis)
        .await
        .unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let _: () = redis::cmd("ZADD")
        .arg(format!("{base}:{set}"))
        .arg(now)
        .arg(id)
        .query_async(redis)
        .await
        .unwrap();
    let mut hset = redis::cmd("HSET");
    hset.arg(format!("{base}:{id}"))
        .arg("finishedOn")
        .arg(now)
        .arg("processedOn")
        .arg(now - 50)
        .arg("attemptsMade")
        .arg(1);
    for (k, v) in fields {
        hset.arg(*k).arg(v);
    }
    let _: () = hset.query_async(redis).await.unwrap();
}

#[tokio::test]
async fn no_read_tool_lets_a_matching_field_through() {
    if skip() {
        return;
    }
    let host = std::env::var("RANCH_REDIS_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = std::env::var("RANCH_REDIS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6390);
    let queue = format!("share-{}", chrono::Utc::now().timestamp_millis());

    let home = tempfile::tempdir().unwrap();
    let data = app_data_dir(home.path());
    std::fs::create_dir_all(&data).unwrap();
    let db = Db::init(&data).await.unwrap();
    let keychain: Arc<dyn KeychainAdapter> = Arc::new(InMemoryKeychain::new());
    let connection = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "share".into(),
            host: host.clone(),
            port,
            db: 0,
            username: None,
            password: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
            ssh_password: None,
            group: GroupSelector::named("Default"),
            environment: EnvironmentSelector::for_color("env-local"),
        },
        db.pool(),
        &keychain,
    )
    .await
    .unwrap();
    // No human to approve the seeding writes in this test.
    let mut safety = get_mcp_safety_settings_inner(db.pool()).await.unwrap();
    safety.approval_mode = ApprovalMode::Off;
    set_mcp_safety_settings_inner(&safety, db.pool())
        .await
        .unwrap();

    let mut redis = redis::Client::open(format!("redis://{host}:{port}"))
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    cleanup(&mut redis, &queue).await;
    let _: () = redis::cmd("HSET")
        .arg(format!("{PREFIX}:{queue}:meta"))
        .arg("version")
        .arg("bullmq:5.81.5")
        .query_async(&mut redis)
        .await
        .unwrap();

    let mut mcp = Mcp::start(home.path());
    let conn = connection.id.clone();
    let queue_ref = json!({ "prefix": PREFIX, "name": queue });
    let payload = json!({
        "customer": { "email": "ann@example.com", "phone": "+33612345678", "name": "Ann" },
        "auth": { "accessToken": "tok_live_123" },
        "lines": [{ "sku": "A-1", "cardNumber": "4111111111111111" }]
    });
    let mut ids = Vec::new();
    for name in ["charge", "refund", "notify"] {
        let added: Value = serde_json::from_str(&mcp.call(
            "add_job",
            json!({ "connectionId": conn, "queueRef": queue_ref, "name": name, "data": payload }),
        ))
        .unwrap();
        ids.push(
            added
                .pointer("/jobId")
                .or_else(|| added.pointer("/id"))
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("no job id in {added}"))
                .to_string(),
        );
    }
    finish(
        &mut redis,
        &queue,
        &ids[0],
        "failed",
        &[
            ("failedReason", "card declined".into()),
            (
                "stacktrace",
                r#"["Error: card declined\n    at charge"]"#.into(),
            ),
        ],
    )
    .await;
    finish(
        &mut redis,
        &queue,
        &ids[1],
        "completed",
        &[(
            "returnvalue",
            json!({ "refund": { "iban": "FR7630006000011234567890189" }, "password": "hunter2" })
                .to_string(),
        )],
    )
    .await;
    let _: () = redis::cmd("HSET")
        .arg(format!("{PREFIX}:{queue}:{}", ids[2]))
        .arg("progress")
        .arg(json!({ "step": 2, "secret": "hunter2" }).to_string())
        .query_async(&mut redis)
        .await
        .unwrap();
    mcp.call(
        "upsert_scheduler",
        json!({
            "connectionId": conn, "queueRef": queue_ref, "schedulerId": "nightly",
            "jobName": "sync", "pattern": "0 3 * * *",
            "data": { "apiToken": "sk_template_999", "region": "eu" }
        }),
    );

    // Every read tool, on everything above.
    let job = |id: &str| json!({ "connectionId": conn, "queueRef": queue_ref, "jobId": id });
    let mut answers: Vec<(String, String)> = Vec::new();
    let mut ask = |mcp: &mut Mcp, tool: &str, args: Value| {
        let text = mcp.call(tool, args);
        answers.push((tool.to_string(), text));
    };
    ask(&mut mcp, "list_connections", json!({}));
    ask(&mut mcp, "list_environments", json!({}));
    ask(&mut mcp, "list_queues", json!({ "connectionId": conn }));
    for state in ["waiting", "failed", "completed"] {
        ask(
            &mut mcp,
            "list_jobs",
            json!({ "connectionId": conn, "queueRef": queue_ref, "state": state }),
        );
    }
    for id in &ids {
        ask(&mut mcp, "inspect_job", job(id));
        ask(&mut mcp, "get_job_logs", job(id));
        ask(&mut mcp, "get_flow", job(id));
    }
    ask(
        &mut mcp,
        "list_failed_jobs",
        json!({ "connectionId": conn }),
    );
    ask(
        &mut mcp,
        "list_schedulers",
        json!({ "connectionId": conn, "queueRef": queue_ref }),
    );
    ask(
        &mut mcp,
        "list_workers",
        json!({ "connectionId": conn, "queueRef": queue_ref }),
    );
    ask(
        &mut mcp,
        "get_queue_metrics",
        json!({ "connectionId": conn, "queueRef": queue_ref }),
    );
    ask(
        &mut mcp,
        "get_connection_metrics",
        json!({ "connectionId": conn }),
    );

    for (tool, text) in &answers {
        for secret in SECRETS {
            assert!(!text.contains(secret), "{tool} leaked {secret}: {text}");
        }
    }
    let detail = &answers
        .iter()
        .find(|(t, a)| t == "inspect_job" && a.contains("charge"))
        .unwrap()
        .1;
    assert!(detail.contains(MASK), "masked, not dropped: {detail}");
    assert!(
        detail.contains("Ann") && detail.contains("A-1"),
        "the rest stays: {detail}"
    );
    assert!(detail.contains("card declined"), "errors stay: {detail}");

    // Names, states and errors only: no payload at all.
    set_kv_setting_inner(db.pool(), AI_SHARE_MODE_KEY, "names")
        .await
        .unwrap();
    let detail: Value = serde_json::from_str(&mcp.call("inspect_job", job(&ids[0]))).unwrap();
    assert_eq!(detail["data"], Value::Null, "{detail}");
    assert!(detail["dataWithheld"].is_string(), "{detail}");
    assert_eq!(detail["name"], "charge");
    assert_eq!(detail["failedReason"], "card declined");
    let schedulers = mcp.call(
        "list_schedulers",
        json!({ "connectionId": conn, "queueRef": queue_ref }),
    );
    assert!(!schedulers.contains("region"), "{schedulers}");

    drop(mcp);
    cleanup(&mut redis, &queue).await;
}
