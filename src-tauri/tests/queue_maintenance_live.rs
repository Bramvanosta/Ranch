//! Live tests for queue maintenance and for the retry /
//! promote scripts (which replaced plain `MULTI/EXEC` writes).
//!
//!   docker compose --profile redis up -d
//!   RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test queue_maintenance_live -- --include-ignored
//!
//! Self-seeded under the `ranch95` prefix. What real BullMQ workers make of
//! the result is `maintenance_compat_live.rs`.

// Same decision as the library (`src/lib.rs`): `RanchError` is wide and typed by design.
#![allow(clippy::result_large_err)]

use ranch_lib::adapters::bullmq_redis::bullmq_schema::QueueRef;
use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::audit::{read_audit_log, ListAuditLogEntriesArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::jobs::{promote_job_inner, retry_job_inner, JobActionArgs};
use ranch_lib::commands::queues::MaintenanceResult;
use ranch_lib::commands::queues::{
    preview_queue_maintenance_inner, run_queue_maintenance_inner, QueueMaintenanceArgs,
};
use ranch_lib::commands::queues::{CleanState, QueueMaintenance};
use ranch_lib::domain::audit::AuditOutcome;
use ranch_lib::domain::audit::Caller;
use ranch_lib::domain::errors::RanchError;
use std::sync::Arc;

mod common;
use common::no_progress;
use common::{redis_target, repo};

const PREFIX: &str = "ranch95";

struct Setup {
    _dir: tempfile::TempDir,
    db: Db,
    kc: Arc<dyn SecretStore>,
    registry: ConnectionRegistry,
    conn_id: String,
    redis: redis::aio::MultiplexedConnection,
}

async fn setup(env_color: &str) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let conn = ranch_lib::commands::connections::add_connection_inner(
        ranch_lib::commands::connections::AddConnectionArgs {
            locked: None,
            name: "test".into(),
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
            group: GroupSelector::named("Default"),
            environment: EnvironmentSelector::for_color(env_color),
        },
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let redis = redis::Client::open(format!(
        "redis://{}:{}",
        redis_target::host(),
        redis_target::port()
    ))
    .unwrap()
    .get_multiplexed_async_connection()
    .await
    .unwrap();
    Setup {
        _dir: dir,
        db,
        kc,
        registry: ConnectionRegistry::new(),
        conn_id: conn.id,
        redis,
    }
}

fn key(queue: &str, suffix: &str) -> String {
    format!("{PREFIX}:{queue}:{suffix}")
}

async fn keys_of(redis: &mut redis::aio::MultiplexedConnection, queue: &str) -> Vec<String> {
    let mut keys: Vec<String> = redis::cmd("KEYS")
        .arg(key(queue, "*"))
        .query_async(redis)
        .await
        .unwrap();
    keys.sort();
    keys
}

async fn cleanup(redis: &mut redis::aio::MultiplexedConnection, queue: &str) {
    let keys = keys_of(redis, queue).await;
    if !keys.is_empty() {
        let _: () = redis::cmd("DEL")
            .arg(keys)
            .query_async(redis)
            .await
            .unwrap();
    }
}

async fn seed_meta(redis: &mut redis::aio::MultiplexedConnection, queue: &str, paused: bool) {
    cleanup(redis, queue).await;
    let mut cmd = redis::cmd("HSET");
    cmd.arg(key(queue, "meta"))
        .arg("version")
        .arg("bullmq:5.81.5");
    if paused {
        cmd.arg("paused").arg("1");
    }
    let _: () = cmd.query_async(redis).await.unwrap();
}

/// A job hash (+ a log list, so orphans show) in the given container.
async fn seed_job(
    redis: &mut redis::aio::MultiplexedConnection,
    queue: &str,
    id: &str,
    container: &str,
    score: i64,
    extra: &[(&str, &str)],
) {
    let mut pipe = redis::pipe();
    let mut hset = redis::cmd("HSET");
    hset.arg(key(queue, id))
        .arg("name")
        .arg("job")
        .arg("data")
        .arg("{}")
        .arg("opts")
        .arg("{}")
        .arg("timestamp")
        .arg(1_000);
    for (k, v) in extra {
        hset.arg(k).arg(v);
    }
    pipe.add_command(hset);
    pipe.cmd("RPUSH")
        .arg(key(queue, &format!("{id}:logs")))
        .arg("a log line");
    match container {
        "wait" | "paused" => {
            pipe.cmd("LPUSH").arg(key(queue, container)).arg(id);
        }
        _ => {
            pipe.cmd("ZADD")
                .arg(key(queue, container))
                .arg(score)
                .arg(id);
        }
    }
    let _: () = pipe.query_async(redis).await.unwrap();
}

fn args(s: &Setup, queue: &str, operation: QueueMaintenance) -> QueueMaintenanceArgs {
    QueueMaintenanceArgs {
        connection_id: s.conn_id.clone(),
        queue_id: QueueRef {
            prefix: PREFIX.into(),
            name: queue.into(),
        }
        .id()
        .into(),
        operation,
    }
}

async fn preview(s: &Setup, queue: &str, operation: QueueMaintenance) -> (u64, u64) {
    let p = preview_queue_maintenance_inner(
        args(s, queue, operation),
        &common::backends(s.db.pool(), &s.kc, &s.registry),
    )
    .await
    .expect("preview");
    (p.affected, p.spared_scheduler_jobs)
}

async fn run(
    s: &Setup,
    queue: &str,
    operation: QueueMaintenance,
) -> Result<MaintenanceResult, RanchError> {
    run_queue_maintenance_inner(
        args(s, queue, operation),
        &common::backends(s.db.pool(), &s.kc, &s.registry),
        no_progress(),
        Caller::User,
    )
    .await
}

async fn audit(s: &Setup, action: &str) -> Vec<ranch_lib::domain::audit::AuditLogEntry> {
    read_audit_log(
        ListAuditLogEntriesArgs {
            from: None,
            to: None,
            connection_id: Some(s.conn_id.clone()),
            action: Some(action.into()),
            source: None,
            offset: 0,
            limit: 100,
        },
        s.db.pool(),
    )
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn drain_removes_what_has_not_started_and_leaves_no_orphan_key() {
    let mut s = setup("env-local").await;
    let q = "drain";
    seed_meta(&mut s.redis, q, false).await;
    for id in ["w1", "w2", "w3"] {
        seed_job(&mut s.redis, q, id, "wait", 0, &[]).await;
    }
    seed_job(&mut s.redis, q, "p1", "paused", 0, &[]).await;
    seed_job(&mut s.redis, q, "pr1", "prioritized", 5, &[]).await;
    seed_job(&mut s.redis, q, "d1", "delayed", 9_000, &[]).await;
    // What drain must leave alone:
    seed_job(&mut s.redis, q, "a1", "active", 0, &[]).await;
    seed_job(&mut s.redis, q, "c1", "completed", 1_000, &[]).await;
    seed_job(&mut s.redis, q, "f1", "failed", 1_000, &[]).await;
    //  - the pending run of a job scheduler (`repeat:<id>:<next>`),
    let _: () = redis::cmd("ZADD")
        .arg(key(q, "repeat"))
        .arg(7_777)
        .arg("nightly")
        .query_async(&mut s.redis)
        .await
        .unwrap();
    seed_job(
        &mut s.redis,
        q,
        "repeat:nightly:7777",
        "delayed",
        7_777 * 4096,
        &[],
    )
    .await;
    //  - a flow child.
    seed_job(
        &mut s.redis,
        q,
        "child",
        "wait",
        0,
        &[("parentKey", "ranch95:other:parent")],
    )
    .await;

    let drain = QueueMaintenance::Drain {
        include_delayed: true,
    };
    assert_eq!(
        preview(&s, q, drain.clone()).await,
        (7, 1),
        "upper bound: the flow child is only seen while running"
    );

    let out = run(&s, q, drain).await.expect("drain");
    assert_eq!(out.action, "drain");
    assert_eq!(out.affected, 6);
    assert_eq!(out.skipped, 2, "scheduler run + flow child");
    assert!(!out.cancelled);

    let left = keys_of(&mut s.redis, q).await;
    let expect: Vec<String> = [
        "a1",
        "a1:logs",
        "active",
        "c1",
        "c1:logs",
        "child",
        "child:logs",
        "completed",
        "delayed",
        "f1",
        "f1:logs",
        "failed",
        "meta",
        "repeat",
        "repeat:nightly:7777",
        "repeat:nightly:7777:logs",
        "wait",
    ]
    .iter()
    .map(|k| key(q, k))
    .collect();
    assert_eq!(left, expect, "no orphan hash / logs, nothing else touched");
    let wait: Vec<String> = redis::cmd("LRANGE")
        .arg(key(q, "wait"))
        .arg(0)
        .arg(-1)
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(wait, ["child"]);

    let rows = audit(&s, "drainQueue").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, AuditOutcome::Success);
    assert_eq!(
        rows[0].outcome_reason.as_deref(),
        Some("6 removed, 2 left in place")
    );

    // Without delayed: nothing left to do, delayed untouched.
    seed_job(&mut s.redis, q, "d2", "delayed", 9_000, &[]).await;
    let drain_no_delayed = QueueMaintenance::Drain {
        include_delayed: false,
    };
    assert_eq!(
        preview(&s, q, drain_no_delayed.clone()).await.0,
        1,
        "the flow child, by count"
    );
    let out = run(&s, q, drain_no_delayed).await.expect("drain");
    assert_eq!((out.affected, out.skipped), (0, 1));
    let d2: i64 = redis::cmd("EXISTS")
        .arg(key(q, "d2"))
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(d2, 1);

    s.registry.shutdown_all().await;
    cleanup(&mut s.redis, q).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn clean_removes_finished_jobs_older_than_the_cutoff_in_batches() {
    let mut s = setup("env-local").await;
    let q = "clean";
    seed_meta(&mut s.redis, q, false).await;
    let now = chrono::Utc::now().timestamp_millis();
    let hour = 3_600_000;
    // 1,200 old completed jobs (> 2 batches) + 3 recent ones + failed jobs.
    {
        let mut pipe = redis::pipe();
        for i in 0..1_200 {
            pipe.cmd("HSET")
                .arg(key(q, &format!("old{i}")))
                .arg("name")
                .arg("job");
            pipe.cmd("ZADD")
                .arg(key(q, "completed"))
                .arg(now - 48 * hour - i)
                .arg(format!("old{i}"));
        }
        let _: () = pipe.query_async(&mut s.redis).await.unwrap();
    }
    for id in ["new1", "new2", "new3"] {
        seed_job(&mut s.redis, q, id, "completed", now - 60_000, &[]).await;
    }
    seed_job(&mut s.redis, q, "f-old", "failed", now - 48 * hour, &[]).await;
    seed_job(
        &mut s.redis,
        q,
        "flow-old",
        "completed",
        now - 48 * hour,
        &[("parentKey", "x:y:z")],
    )
    .await;

    let day_old = QueueMaintenance::Clean {
        state: CleanState::Completed,
        older_than_ms: 24 * hour as u64,
    };
    assert_eq!(preview(&s, q, day_old.clone()).await.0, 1_201);
    let out = run(&s, q, day_old).await.expect("clean");
    assert_eq!((out.affected, out.skipped), (1_200, 1));

    let completed: Vec<String> = redis::cmd("ZRANGE")
        .arg(key(q, "completed"))
        .arg(0)
        .arg(-1)
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(
        completed.len(),
        4,
        "3 recent + the flow member: {completed:?}"
    );
    let orphans: Vec<String> = keys_of(&mut s.redis, q)
        .await
        .into_iter()
        .filter(|k| k.contains(":old"))
        .collect();
    assert!(orphans.is_empty(), "{orphans:?}");
    let failed: i64 = redis::cmd("ZCARD")
        .arg(key(q, "failed"))
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(failed, 1, "the other state is not touched");
    // BullMQ's `cleaned` event, one per batch that removed something.
    let events: Vec<redis::Value> = redis::cmd("XRANGE")
        .arg(key(q, "events"))
        .arg("-")
        .arg("+")
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(events.len(), 3, "500 + 500 + 200");

    // older_than 0 = everything of that state.
    let all_failed = QueueMaintenance::Clean {
        state: CleanState::Failed,
        older_than_ms: 0,
    };
    assert_eq!(preview(&s, q, all_failed.clone()).await.0, 1);
    assert_eq!(run(&s, q, all_failed).await.expect("clean").affected, 1);

    let rows = audit(&s, "cleanQueue").await;
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|r| r.outcome_reason.as_deref()
        == Some("1200 removed, 1 left in place (completed older than 86400000 ms)")));

    s.registry.shutdown_all().await;
    cleanup(&mut s.redis, q).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn retry_all_failed_requeues_every_job_wakes_workers_and_erases_the_failure() {
    let mut s = setup("env-local").await;
    let q = "retryall";
    seed_meta(&mut s.redis, q, false).await;
    let now = chrono::Utc::now().timestamp_millis();
    {
        let mut pipe = redis::pipe();
        for i in 0..1_100 {
            pipe.cmd("HSET")
                .arg(key(q, &format!("j{i}")))
                .arg("name")
                .arg("job")
                .arg("failedReason")
                .arg("boom")
                .arg("processedOn")
                .arg(now - 2_000)
                .arg("finishedOn")
                .arg(now - 1_000)
                .arg("atm")
                .arg(1);
            pipe.cmd("ZADD")
                .arg(key(q, "failed"))
                .arg(now - 1_000)
                .arg(format!("j{i}"));
        }
        // A dangling id: in the set, hash gone.
        pipe.cmd("ZADD")
            .arg(key(q, "failed"))
            .arg(now - 1_000)
            .arg("ghost");
        let _: () = pipe.query_async(&mut s.redis).await.unwrap();
    }

    assert_eq!(
        preview(&s, q, QueueMaintenance::RetryAllFailed).await.0,
        1_101
    );
    let out = run(&s, q, QueueMaintenance::RetryAllFailed)
        .await
        .expect("retry all");
    assert_eq!(out.action, "retry");
    assert_eq!((out.affected, out.skipped), (1_100, 1));

    let (failed, wait): (i64, i64) = redis::pipe()
        .cmd("ZCARD")
        .arg(key(q, "failed"))
        .cmd("LLEN")
        .arg(key(q, "wait"))
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!((failed, wait), (0, 1_100));
    let fields: Vec<String> = redis::cmd("HKEYS")
        .arg(key(q, "j7"))
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert!(
        !fields
            .iter()
            .any(|f| f == "failedReason" || f == "finishedOn" || f == "processedOn"),
        "{fields:?}"
    );
    assert!(
        fields.iter().any(|f| f == "atm"),
        "attempts are kept, as BullMQ does"
    );
    let marker: Vec<String> = redis::cmd("ZRANGE")
        .arg(key(q, "marker"))
        .arg(0)
        .arg(-1)
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(marker, ["0"], "idle workers are woken");
    let events: i64 = redis::cmd("XLEN")
        .arg(key(q, "events"))
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(events, 1_100, "one `waiting` event per job");

    let rows = audit(&s, "retryAllFailed").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].outcome_reason.as_deref(),
        Some("1100 retried, 1 left in place")
    );

    s.registry.shutdown_all().await;
    cleanup(&mut s.redis, q).await;
}

/// The bug the retry / promote scripts fix: on a paused queue the old
/// `MULTI/EXEC` pushed into `wait`; BullMQ's resume then `RENAME`s `paused`
/// over `wait` and the job is in no list at all.
#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn retry_and_promote_respect_a_paused_queue_and_a_job_priority() {
    let mut s = setup("env-local").await;
    let q = "pausedmoves";
    seed_meta(&mut s.redis, q, true).await;
    seed_job(
        &mut s.redis,
        q,
        "f1",
        "failed",
        1_000,
        &[("failedReason", "boom")],
    )
    .await;
    seed_job(
        &mut s.redis,
        q,
        "d1",
        "delayed",
        9_000,
        &[("delay", "5000")],
    )
    .await;
    seed_job(
        &mut s.redis,
        q,
        "d2",
        "delayed",
        9_001,
        &[("delay", "5000"), ("priority", "3")],
    )
    .await;
    let job = |id: &str| JobActionArgs {
        connection_id: s.conn_id.clone(),
        queue_id: QueueRef {
            prefix: PREFIX.into(),
            name: q.into(),
        }
        .id()
        .into(),
        job_id: id.into(),
    };

    retry_job_inner(
        job("f1"),
        &common::backends(s.db.pool(), &s.kc, &s.registry),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .expect("retry");
    promote_job_inner(
        job("d1"),
        &common::backends(s.db.pool(), &s.kc, &s.registry),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .expect("promote");
    promote_job_inner(
        job("d2"),
        &common::backends(s.db.pool(), &s.kc, &s.registry),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .expect("promote");

    let paused: Vec<String> = redis::cmd("LRANGE")
        .arg(key(q, "paused"))
        .arg(0)
        .arg(-1)
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(paused, ["d1", "f1"], "parked, not pushed into `wait`");
    let prioritized: Vec<String> = redis::cmd("ZRANGE")
        .arg(key(q, "prioritized"))
        .arg(0)
        .arg(-1)
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(
        prioritized,
        ["d2"],
        "a promoted job with a priority keeps it"
    );
    for k in ["wait", "marker"] {
        let exists: i64 = redis::cmd("EXISTS")
            .arg(key(q, k))
            .query_async(&mut s.redis)
            .await
            .unwrap();
        assert_eq!(exists, 0, "{k} must not exist on a paused queue");
    }
    let delay: String = redis::cmd("HGET")
        .arg(key(q, "d1"))
        .arg("delay")
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(delay, "0");

    // Wrong state is still refused, by the script this time.
    let err = retry_job_inner(
        job("d1"),
        &common::backends(s.db.pool(), &s.kc, &s.registry),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .expect_err("not failed");
    assert!(matches!(err, RanchError::InvalidJobState { .. }), "{err:?}");

    s.registry.shutdown_all().await;
    cleanup(&mut s.redis, q).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn a_locked_connection_refuses_every_maintenance_operation_before_redis() {
    let mut s = setup("env-production").await;
    let q = "locked";
    seed_meta(&mut s.redis, q, false).await;
    seed_job(&mut s.redis, q, "w1", "wait", 0, &[]).await;
    seed_job(&mut s.redis, q, "c1", "completed", 1_000, &[]).await;
    seed_job(&mut s.redis, q, "f1", "failed", 1_000, &[]).await;
    let before = keys_of(&mut s.redis, q).await;

    for op in [
        QueueMaintenance::Drain {
            include_delayed: true,
        },
        QueueMaintenance::Clean {
            state: CleanState::Completed,
            older_than_ms: 0,
        },
        QueueMaintenance::RetryAllFailed,
    ] {
        // The preview is a read: allowed.
        assert_eq!(preview(&s, q, op.clone()).await.0, 1);
        let err = run(&s, q, op).await.expect_err("locked");
        assert!(
            matches!(err, RanchError::ConnectionLocked { .. }),
            "{err:?}"
        );
    }
    assert_eq!(keys_of(&mut s.redis, q).await, before);

    s.registry.shutdown_all().await;
    cleanup(&mut s.redis, q).await;
}
