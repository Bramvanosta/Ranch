//! Every MCP write tool refuses a locked connection before touching Redis.
//!
//! The list of tools comes from the catalogue (`application::catalogue`),
//! not from this file: every write tool through the write pipeline. One
//! added without an entry in [`input_for`] fails here until someone says
//! how to call it, and then it is checked. A write tool outside the
//! pipeline says why in the catalogue.
//! The connection is open to agent writes and approvals are off, so the
//! session lock is the only thing standing between the agent and Redis.
//!
//! Against Ranch's own Redis (CI runs it):
//!
//!   RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test lock_sentinel_live -- --include-ignored

use std::sync::Arc;

use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::catalogue::{self, Kind, Trail};
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::mcp_safety::set_connection_mcp_readonly_inner;
use ranch_lib::commands::settings::set_kv_setting_inner;
use ranch_lib::core::mcp::approval::APPROVAL_MODE_KEY;
use serde_json::{json, Value};

mod common;
use common::mcp_stdio::{app_data_dir, Mcp};
use common::{kv, redis_target, repo};

/// How to call each write tool on `queue`. A tool missing here fails the test.
fn input_for(tool: &str, connection: &str, queue: &str) -> Option<Value> {
    let queue_id = json!(format!("bull:{queue}"));
    let base = json!({ "connectionId": connection, "queueId": queue_id });
    let with = |extra: Value| {
        let mut v = base.clone();
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        v
    };
    Some(match tool {
        "retry_job" | "remove_job" | "promote_job" => with(json!({ "jobId": "1" })),
        "bulk_retry_jobs" | "bulk_remove_jobs" | "bulk_promote_jobs" => {
            with(json!({ "jobIds": ["1", "2"] }))
        }
        "pause_queue" | "resume_queue" => base,
        "pause_scheduler" | "resume_scheduler" | "remove_scheduler" => {
            with(json!({ "schedulerKey": "nightly" }))
        }
        "add_job" => with(json!({ "name": "send-email", "data": { "to": "a@b.c" } })),
        "drain_queue" => base,
        "clean_queue" => with(json!({ "state": "completed", "olderThanMs": 0 })),
        "upsert_scheduler" => with(json!({
            "schedulerId": "nightly",
            "jobName": "report",
            "pattern": "0 3 * * *"
        })),
        _ => return None,
    })
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn every_write_tool_refuses_a_locked_connection_before_redis() {
    let queue = format!("lock-sentinel-{}", chrono::Utc::now().timestamp_millis());
    let mut redis = redis_target::connect().await;
    // A queue with a waiting job, so that nothing fails for want of one.
    let _: () = redis::cmd("HSET")
        .arg(format!("bull:{queue}:meta"))
        .arg("version")
        .arg("bullmq:5.76.6")
        .query_async(&mut redis)
        .await
        .unwrap();
    let _: () = redis::cmd("HSET")
        .arg(format!("bull:{queue}:1"))
        .arg("name")
        .arg("send-email")
        .arg("data")
        .arg("{}")
        .arg("timestamp")
        .arg(chrono::Utc::now().timestamp_millis())
        .query_async(&mut redis)
        .await
        .unwrap();
    let _: () = redis::cmd("LPUSH")
        .arg(format!("bull:{queue}:wait"))
        .arg("1")
        .query_async(&mut redis)
        .await
        .unwrap();
    let pattern = format!("bull:{queue}:*");
    let before = redis_target::keys_of(&mut redis, &pattern).await;

    // A production connection (locked from the start), opened to agent
    // writes, with approvals off: only the lock can say no.
    let home = tempfile::tempdir().unwrap();
    let data = app_data_dir(home.path());
    std::fs::create_dir_all(&data).unwrap();
    let db = Db::init(&data).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "prod".into(),
            backend: "bullmq-redis".into(),
            endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
                host: redis_target::host(),
                port: redis_target::port(),
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
            environment: EnvironmentSelector::named("Production", "env-production"),
        },
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    set_connection_mcp_readonly_inner(&conn.id, Some(false), &repo(db.pool()))
        .await
        .unwrap();
    set_kv_setting_inner(&kv(db.pool()), APPROVAL_MODE_KEY, "off")
        .await
        .unwrap();

    let mut mcp = Mcp::start(home.path());
    let mut checked = Vec::new();
    for (tool, op) in catalogue::tools() {
        if op.kind != Kind::Write(Trail::Pipeline) {
            continue;
        }
        let input = input_for(tool, &conn.id, &queue).unwrap_or_else(|| {
            panic!("write tool `{tool}` has no entry in input_for: add one so its lock check is tested")
        });
        let kind = mcp.error_kind(tool, input);
        assert_eq!(kind, "connectionLocked", "{tool} on a locked connection");
        checked.push(tool);
    }
    assert!(checked.len() >= 15, "only {checked:?} were checked");

    // Nothing reached Redis.
    assert_eq!(redis_target::keys_of(&mut redis, &pattern).await, before);
    redis_target::delete_matching(&mut redis, &pattern).await;
}
