//! What every MCP write tool does today, end to end: the change in Redis
//! and the audit row it leaves, signed by the agent.
//!
//! The list of tools comes from the catalogue (`application::catalogue`):
//! every write tool through the write pipeline. One added without a case
//! here fails until someone describes what it should do. A write tool
//! outside the pipeline says why in the catalogue. The connection is open to agent writes and approvals are
//! off, so each call runs straight through.
//!
//! Against Ranch's own Redis (CI runs it):
//!
//!   RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test mcp_writes_live -- --include-ignored

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
use ranch_lib::domain::audit::{AuditOutcome, AuditSource, AuditTarget};
use ranch_lib::domain::queues::QueueId;
use redis::aio::MultiplexedConnection;
use serde_json::{json, Value};

mod common;
use common::bullmq_seed::{exists, list_ids, seed_job, seed_queue, zset_ids};
use common::mcp_stdio::{app_data_dir, Mcp};
use common::{audit, redis_target};
use common::{kv, repo};

fn queue_id(queue: &str) -> Value {
    json!(format!("bull:{queue}"))
}

/// Put the queue in the state the tool acts on.
async fn prepare(
    tool: &str,
    redis: &mut MultiplexedConnection,
    mcp: &mut Mcp,
    conn: &str,
    queue: &str,
) {
    match tool {
        "retry_job" => seed_job(redis, queue, "1", "failed").await,
        "remove_job" => seed_job(redis, queue, "1", "waiting").await,
        "promote_job" => seed_job(redis, queue, "1", "delayed").await,
        "bulk_retry_jobs" => {
            seed_job(redis, queue, "1", "failed").await;
            seed_job(redis, queue, "2", "failed").await;
        }
        "bulk_remove_jobs" => {
            seed_job(redis, queue, "1", "waiting").await;
            seed_job(redis, queue, "2", "waiting").await;
        }
        "bulk_promote_jobs" => {
            seed_job(redis, queue, "1", "delayed").await;
            seed_job(redis, queue, "2", "delayed").await;
        }
        "pause_queue" | "add_job" | "upsert_scheduler" => seed_queue(redis, queue).await,
        "resume_queue" => {
            seed_job(redis, queue, "1", "waiting").await;
            mcp.call(
                "pause_queue",
                json!({ "connectionId": conn, "queueId": queue_id(queue) }),
            );
        }
        "drain_queue" => {
            seed_job(redis, queue, "1", "waiting").await;
            seed_job(redis, queue, "2", "waiting").await;
        }
        "clean_queue" => seed_job(redis, queue, "1", "completed").await,
        "pause_scheduler" | "resume_scheduler" | "remove_scheduler" => {
            seed_queue(redis, queue).await;
            mcp.call("upsert_scheduler", upsert_input(conn, queue));
            if tool == "resume_scheduler" {
                mcp.call("pause_scheduler", scheduler_input(conn, queue));
            }
        }
        other => panic!("write tool `{other}` has no case in prepare: describe what it does"),
    }
}

fn upsert_input(conn: &str, queue: &str) -> Value {
    json!({
        "connectionId": conn,
        "queueId": queue_id(queue),
        "schedulerId": "nightly",
        "jobName": "report",
        "pattern": "0 3 * * *"
    })
}

fn scheduler_input(conn: &str, queue: &str) -> Value {
    json!({ "connectionId": conn, "queueId": queue_id(queue), "schedulerKey": "nightly" })
}

fn input(tool: &str, conn: &str, queue: &str) -> Value {
    let base = json!({ "connectionId": conn, "queueId": queue_id(queue) });
    let with = |extra: Value| {
        let mut v = base.clone();
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        v
    };
    match tool {
        "retry_job" | "remove_job" | "promote_job" => with(json!({ "jobId": "1" })),
        "bulk_retry_jobs" | "bulk_remove_jobs" | "bulk_promote_jobs" => {
            with(json!({ "jobIds": ["1", "2"] }))
        }
        "pause_queue" | "resume_queue" | "drain_queue" => base,
        "add_job" => with(json!({ "name": "send-email", "data": { "to": "a@b.c" } })),
        "clean_queue" => with(json!({ "state": "completed", "olderThanMs": 0 })),
        "upsert_scheduler" => upsert_input(conn, queue),
        "pause_scheduler" | "resume_scheduler" | "remove_scheduler" => scheduler_input(conn, queue),
        other => panic!("write tool `{other}` has no case in input"),
    }
}

/// The change in Redis the tool is for.
async fn assert_effect(tool: &str, redis: &mut MultiplexedConnection, queue: &str) {
    let key = |suffix: &str| format!("bull:{queue}:{suffix}");
    match tool {
        "retry_job" => {
            assert!(zset_ids(redis, queue, "failed").await.is_empty());
            assert_eq!(list_ids(redis, queue, "wait").await, ["1"]);
        }
        "remove_job" => assert!(!exists(redis, &key("1")).await),
        "promote_job" => {
            assert!(zset_ids(redis, queue, "delayed").await.is_empty());
            assert_eq!(list_ids(redis, queue, "wait").await, ["1"]);
        }
        "bulk_retry_jobs" => {
            assert!(zset_ids(redis, queue, "failed").await.is_empty());
            assert_eq!(list_ids(redis, queue, "wait").await.len(), 2);
        }
        "bulk_remove_jobs" => {
            assert!(!exists(redis, &key("1")).await && !exists(redis, &key("2")).await);
        }
        "bulk_promote_jobs" => {
            assert!(zset_ids(redis, queue, "delayed").await.is_empty());
            assert_eq!(list_ids(redis, queue, "wait").await.len(), 2);
        }
        "pause_queue" => assert!(meta_paused(redis, queue).await),
        "resume_queue" => {
            assert!(!meta_paused(redis, queue).await);
            assert_eq!(list_ids(redis, queue, "wait").await, ["1"]);
        }
        "add_job" => {
            let ids = list_ids(redis, queue, "wait").await;
            assert_eq!(ids.len(), 1);
            let name: String = redis::cmd("HGET")
                .arg(key(&ids[0]))
                .arg("name")
                .query_async(redis)
                .await
                .unwrap();
            assert_eq!(name, "send-email");
        }
        "drain_queue" => {
            assert!(list_ids(redis, queue, "wait").await.is_empty());
            assert!(!exists(redis, &key("1")).await);
        }
        "clean_queue" => {
            assert!(zset_ids(redis, queue, "completed").await.is_empty());
            assert!(!exists(redis, &key("1")).await);
        }
        "upsert_scheduler" | "resume_scheduler" => {
            assert_eq!(zset_ids(redis, queue, "repeat").await, ["nightly"]);
        }
        "pause_scheduler" => {
            let paused: Option<String> = redis::cmd("HGET")
                .arg(key("repeat:nightly"))
                .arg("paused")
                .query_async(redis)
                .await
                .unwrap();
            assert!(paused.is_some(), "the scheduler hash carries its pause");
        }
        "remove_scheduler" => {
            assert!(zset_ids(redis, queue, "repeat").await.is_empty());
            assert!(!exists(redis, &key("repeat:nightly")).await);
        }
        other => panic!("write tool `{other}` has no case in assert_effect"),
    }
}

/// BullMQ reads a queue as paused when its meta has a `paused` field.
async fn meta_paused(redis: &mut MultiplexedConnection, queue: &str) -> bool {
    redis::cmd("HEXISTS")
        .arg(format!("bull:{queue}:meta"))
        .arg("paused")
        .query_async::<i64>(redis)
        .await
        .unwrap()
        == 1
}

/// The action and target the audit row names.
fn audited_as(tool: &str, queue: &str) -> (&'static str, AuditTarget) {
    let id = QueueId::new(format!("bull:{queue}"));
    let job = || AuditTarget::job(&id, queue, "1");
    let bulk = || AuditTarget::bulk_jobs(&id, queue, 2);
    let whole = || AuditTarget::queue(&id, queue);
    let scheduler = || AuditTarget::scheduler(&id, queue, "nightly");
    match tool {
        "retry_job" => ("retryJob", job()),
        "remove_job" => ("removeJob", job()),
        "promote_job" => ("promoteJob", job()),
        "bulk_retry_jobs" => ("retryJob", bulk()),
        "bulk_remove_jobs" => ("removeJob", bulk()),
        "bulk_promote_jobs" => ("promoteJob", bulk()),
        "pause_queue" => ("pauseQueue", whole()),
        "resume_queue" => ("resumeQueue", whole()),
        // The job it created: the first id of a fresh queue.
        "add_job" => ("addJob", job()),
        "drain_queue" => ("drainQueue", whole()),
        "clean_queue" => ("cleanQueue", whole()),
        "upsert_scheduler" => ("upsertScheduler", scheduler()),
        "pause_scheduler" => ("pauseScheduler", scheduler()),
        "resume_scheduler" => ("resumeScheduler", scheduler()),
        "remove_scheduler" => ("removeScheduler", scheduler()),
        other => panic!("write tool `{other}` has no case in audited_as"),
    }
}

/// A local connection open to agent writes, approvals off; `ranch-mcp`
/// on its database. Keep the directory alive as long as the server.
async fn agent_on_a_local_connection() -> (tempfile::TempDir, Db, String, Mcp) {
    let home = tempfile::tempdir().unwrap();
    let data = app_data_dir(home.path());
    std::fs::create_dir_all(&data).unwrap();
    let db = Db::init(&data).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "local".into(),
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
            environment: EnvironmentSelector::named("Local", "env-local"),
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

    let mcp = Mcp::start(home.path());
    (home, db, conn.id, mcp)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn every_write_tool_changes_redis_and_is_audited_as_the_agent() {
    let run = chrono::Utc::now().timestamp_millis();
    let mut redis = redis_target::connect().await;

    let (_home, db, conn_id, mut mcp) = agent_on_a_local_connection().await;
    let mut checked = Vec::new();
    for (tool, op) in catalogue::tools() {
        if op.kind != Kind::Write(Trail::Pipeline) {
            continue;
        }
        let queue = format!("mcp-writes-{tool}-{run}");
        let pattern = format!("bull:{queue}:*");
        redis_target::delete_matching(&mut redis, &pattern).await;
        prepare(tool, &mut redis, &mut mcp, &conn_id, &queue).await;

        mcp.call(tool, input(tool, &conn_id, &queue));
        assert_effect(tool, &mut redis, &queue).await;

        let (action, target) = audited_as(tool, &queue);
        let rows = audit::rows(db.pool(), &conn_id).await;
        let row = rows
            .iter()
            .find(|r| r.action.as_str() == action && r.target == target)
            .unwrap_or_else(|| panic!("{tool}: no `{action}` row for {target:?} in {rows:#?}"));
        assert_eq!(row.source, AuditSource::Agent, "{tool}");
        assert_eq!(row.agent_client.as_deref(), Some("ranch-tests"), "{tool}");
        assert_eq!(row.agent_transport.as_deref(), Some("stdio"), "{tool}");
        assert_eq!(
            row.outcome,
            AuditOutcome::Success,
            "{tool}: {:?}",
            row.outcome_reason
        );
        assert_eq!(
            row.connection_id.as_deref(),
            Some(conn_id.as_str()),
            "{tool}"
        );
        assert_eq!(row.env_color.as_deref(), Some("env-local"), "{tool}");
        assert_eq!(row.approval_id, None, "{tool}: approvals are off");

        redis_target::delete_matching(&mut redis, &pattern).await;
        checked.push(tool);
    }
    assert!(checked.len() >= 15, "only {checked:?} were checked");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn retrying_every_failed_job_is_one_retry_all_row() {
    let queue = format!(
        "mcp-writes-all-failed-{}",
        chrono::Utc::now().timestamp_millis()
    );
    let pattern = format!("bull:{queue}:*");
    let mut redis = redis_target::connect().await;
    let (_home, db, conn_id, mut mcp) = agent_on_a_local_connection().await;
    for id in ["1", "2", "3"] {
        seed_job(&mut redis, &queue, id, "failed").await;
    }

    mcp.call(
        "bulk_retry_jobs",
        json!({ "connectionId": conn_id, "queueId": queue_id(&queue), "allFailed": true }),
    );

    assert!(zset_ids(&mut redis, &queue, "failed").await.is_empty());
    assert_eq!(list_ids(&mut redis, &queue, "wait").await.len(), 3);
    let rows = audit::rows(db.pool(), &conn_id).await;
    assert_eq!(rows.len(), 1, "{rows:#?}");
    assert_eq!(rows[0].action.as_str(), "retryAllFailed");
    assert_eq!(
        rows[0].target,
        AuditTarget::queue(&QueueId::new(format!("bull:{queue}")), &queue)
    );
    assert_eq!(rows[0].source, AuditSource::Agent);
    assert_eq!(rows[0].outcome, AuditOutcome::Success);
    redis_target::delete_matching(&mut redis, &pattern).await;
}
