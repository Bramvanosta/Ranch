//! What agents may see of job data, end to end: the real
//! `ranch-mcp`, driven over stdio, answers every read tool on jobs whose
//! payload, result, progress and scheduler template hold nested sensitive
//! fields; with the default policy (mask) none of their values leaks, and
//! with "names, states and errors only" no payload is sent at all.
//!
//! Needs Ranch's own Redis: `RANCH_LOCAL_REDIS=1` and
//! `docker compose --profile redis up -d` (port 6390).

use std::sync::Arc;

use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::mcp_safety::ApprovalMode;
use ranch_lib::commands::mcp_safety::{
    get_mcp_safety_settings_inner, set_mcp_safety_settings_inner,
};
use ranch_lib::commands::settings::set_kv_setting_inner;
use ranch_lib::core::mcp::share::{AI_SHARE_MODE_KEY, MASK};
use serde_json::{json, Value};

mod common;
use common::mcp_stdio::{app_data_dir, Mcp};
use common::{kv, repo};

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
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn no_read_tool_lets_a_matching_field_through() {
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
    let keychain: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let connection = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "share".into(),
            backend: "bullmq-redis".into(),
            endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
                host: host.clone(),
                port,
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
    // No human to approve the seeding writes in this test.
    let mut safety = get_mcp_safety_settings_inner(&kv(db.pool())).await.unwrap();
    safety.approval_mode = ApprovalMode::Off;
    set_mcp_safety_settings_inner(&safety, &kv(db.pool()))
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
    let queue_id = json!(format!("{PREFIX}:{queue}"));
    let payload = json!({
        "customer": { "email": "ann@example.com", "phone": "+33612345678", "name": "Ann" },
        "auth": { "accessToken": "tok_live_123" },
        "lines": [{ "sku": "A-1", "cardNumber": "4111111111111111" }]
    });
    let mut ids = Vec::new();
    for name in ["charge", "refund", "notify"] {
        let added: Value = serde_json::from_str(&mcp.call(
            "add_job",
            json!({ "connectionId": conn, "queueId": queue_id, "name": name, "data": payload }),
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
            "connectionId": conn, "queueId": queue_id, "schedulerId": "nightly",
            "jobName": "sync", "pattern": "0 3 * * *",
            "data": { "apiToken": "sk_template_999", "region": "eu" }
        }),
    );

    // Every read tool, on everything above.
    let job = |id: &str| json!({ "connectionId": conn, "queueId": queue_id, "jobId": id });
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
            json!({ "connectionId": conn, "queueId": queue_id, "state": state }),
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
        json!({ "connectionId": conn, "queueId": queue_id }),
    );
    ask(
        &mut mcp,
        "list_workers",
        json!({ "connectionId": conn, "queueId": queue_id }),
    );
    ask(
        &mut mcp,
        "get_queue_metrics",
        json!({ "connectionId": conn, "queueId": queue_id }),
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
    set_kv_setting_inner(&kv(db.pool()), AI_SHARE_MODE_KEY, "names")
        .await
        .unwrap();
    let detail: Value = serde_json::from_str(&mcp.call("inspect_job", job(&ids[0]))).unwrap();
    assert_eq!(detail["data"], Value::Null, "{detail}");
    assert!(detail["dataWithheld"].is_string(), "{detail}");
    assert_eq!(detail["name"], "charge");
    assert_eq!(detail["failedReason"], "card declined");
    let schedulers = mcp.call(
        "list_schedulers",
        json!({ "connectionId": conn, "queueId": queue_id }),
    );
    assert!(!schedulers.contains("region"), "{schedulers}");

    drop(mcp);
    cleanup(&mut redis, &queue).await;
}
