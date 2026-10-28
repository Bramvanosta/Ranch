//! Live tests for job scheduler create / edit / delete / pause / resume
//! what lands in Redis. That a real BullMQ worker then
//! receives the jobs on time is `scheduler_compat_live.rs`.
//!
//!   docker compose --profile redis up -d
//!   RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test scheduler_live -- --include-ignored
//!
//! Self-seeded under the `ranch96` prefix.

// Same decision as the library (`src/lib.rs`): `RanchError` is wide and typed by design.
#![allow(clippy::result_large_err)]

use ranch_lib::adapters::bullmq_redis::bullmq_schema::QueueRef;
use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::audit::{read_audit_log, ListAuditLogEntriesArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::queues::SchedulerSaved;
use ranch_lib::commands::queues::{
    list_cron_jobs_inner, pause_scheduler_inner, remove_scheduler_inner, resume_scheduler_inner,
    upsert_scheduler_inner, ListCronJobsArgs, SchedulerActionArgs, UpsertSchedulerArgs,
};
use ranch_lib::commands::queues::{Schedule, SchedulerSpec};
use ranch_lib::domain::audit::Caller;
use ranch_lib::domain::errors::RanchError;
use std::collections::HashMap;
use std::sync::Arc;

mod common;
use common::{redis_target, repo};

const PREFIX: &str = "ranch96";

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

async fn seed_meta(redis: &mut redis::aio::MultiplexedConnection, queue: &str) {
    cleanup(redis, queue).await;
    let _: () = redis::cmd("HSET")
        .arg(key(queue, "meta"))
        .arg("version")
        .arg("bullmq:5.81.5")
        .query_async(redis)
        .await
        .unwrap();
}

fn qref(queue: &str) -> QueueRef {
    QueueRef {
        prefix: PREFIX.into(),
        name: queue.into(),
    }
}

async fn upsert(
    s: &Setup,
    queue: &str,
    id: &str,
    schedule: Schedule,
    data: &str,
) -> Result<SchedulerSaved, RanchError> {
    upsert_scheduler_inner(
        UpsertSchedulerArgs {
            connection_id: s.conn_id.clone(),
            queue_id: qref(queue).id().into(),
            scheduler: SchedulerSpec {
                id: id.into(),
                job_name: "build-report".into(),
                schedule,
                data: data.into(),
            },
        },
        &common::backends(s.db.pool(), &s.kc, &s.registry),
        Caller::User,
    )
    .await
}

fn action(s: &Setup, queue: &str, id: &str) -> SchedulerActionArgs {
    SchedulerActionArgs {
        connection_id: s.conn_id.clone(),
        queue_id: qref(queue).id().into(),
        scheduler_key: id.into(),
    }
}

async fn hash(redis: &mut redis::aio::MultiplexedConnection, k: String) -> HashMap<String, String> {
    redis::cmd("HGETALL")
        .arg(k)
        .query_async(redis)
        .await
        .unwrap()
}

async fn audit_count(s: &Setup, action: &str) -> usize {
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
    .len()
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn a_cron_scheduler_is_written_the_way_bullmq_writes_it() {
    let mut s = setup("env-local").await;
    let q = "cron";
    seed_meta(&mut s.redis, q).await;

    let saved = upsert(
        &s,
        q,
        "nightly",
        Schedule::Pattern {
            pattern: "0 3 * * *".into(),
            tz: Some("Europe/Paris".into()),
        },
        r#"{ "kind": "summary" }"#,
    )
    .await
    .expect("upsert");
    assert_eq!(saved.scheduler_id, "nightly");
    assert_eq!(
        saved.job_id,
        format!("repeat:nightly:{}", saved.next_run_ms)
    );
    let now = chrono::Utc::now().timestamp_millis();
    assert!(saved.next_run_ms > now && saved.next_run_ms <= now + 24 * 3_600_000);

    // repeat zset: score = next run.
    let score: f64 = redis::cmd("ZSCORE")
        .arg(key(q, "repeat"))
        .arg("nightly")
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(score as i64, saved.next_run_ms);

    // The scheduler hash: same fields as one made by `upsertJobScheduler`.
    let sched = hash(&mut s.redis, key(q, "repeat:nightly")).await;
    assert_eq!(sched["name"], "build-report");
    assert_eq!(sched["pattern"], "0 3 * * *");
    assert_eq!(sched["tz"], "Europe/Paris");
    assert_eq!(sched["data"], r#"{"kind":"summary"}"#);
    assert_eq!(sched["ic"], "1");
    assert_eq!(sched["offset"], "0");
    assert!(!sched.contains_key("every"));

    // The pending job: delayed until the run, carrying what the worker needs
    // to compute the run after it.
    let job = hash(&mut s.redis, key(q, &saved.job_id)).await;
    assert_eq!(job["name"], "build-report");
    assert_eq!(job["data"], r#"{"kind":"summary"}"#);
    assert_eq!(job["rjk"], "nightly");
    assert_eq!(job["priority"], "0");
    let opts: serde_json::Value = serde_json::from_str(&job["opts"]).unwrap();
    assert_eq!(opts["repeat"]["pattern"], "0 3 * * *");
    assert_eq!(opts["repeat"]["tz"], "Europe/Paris");
    assert_eq!(opts["repeat"]["count"], 1);
    assert_eq!(opts["repeatJobKey"], "nightly");
    assert_eq!(opts["jobId"], saved.job_id.as_str());
    assert_eq!(opts["prevMillis"], saved.next_run_ms);
    let delayed_score: f64 = redis::cmd("ZSCORE")
        .arg(key(q, "delayed"))
        .arg(&saved.job_id)
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!((delayed_score / 4096.0) as i64, saved.next_run_ms);

    // Ranch's own list reads it back.
    let listed = list_cron_jobs_inner(
        ListCronJobsArgs {
            connection_id: s.conn_id.clone(),
            queue_id: qref(q).id().into(),
        },
        &common::backends(s.db.pool(), &s.kc, &s.registry),
    )
    .await
    .expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].key, "nightly");
    assert_eq!(listed[0].next_run, saved.next_run_ms);
    assert!(!listed[0].paused);

    assert_eq!(audit_count(&s, "upsertScheduler").await, 1);
    s.registry.shutdown_all().await;
    cleanup(&mut s.redis, q).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn editing_replaces_the_pending_job_and_keeps_the_iteration_count() {
    let mut s = setup("env-local").await;
    let q = "edit";
    seed_meta(&mut s.redis, q).await;

    // An `every` scheduler fires now: its first job goes to `wait`.
    let first = upsert(&s, q, "tick", Schedule::Every { every_ms: 60_000 }, "{}")
        .await
        .expect("create");
    let wait: Vec<String> = redis::cmd("LRANGE")
        .arg(key(q, "wait"))
        .arg(0)
        .arg(-1)
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(wait, std::slice::from_ref(&first.job_id));
    let sched = hash(&mut s.redis, key(q, "repeat:tick")).await;
    assert_eq!(sched["every"], "60000");
    assert!(
        !sched.contains_key("data"),
        "an empty template is not stored, as upstream"
    );
    // Pretend it already ran 7 times.
    let _: () = redis::cmd("HSET")
        .arg(key(q, "repeat:tick"))
        .arg("ic")
        .arg(7)
        .query_async(&mut s.redis)
        .await
        .unwrap();

    // Edit: another interval, a template.
    let second = upsert(
        &s,
        q,
        "tick",
        Schedule::Every { every_ms: 5_000 },
        r#"{"v":2}"#,
    )
    .await
    .expect("edit");
    assert_ne!(second.job_id, first.job_id);
    let old_exists: i64 = redis::cmd("EXISTS")
        .arg(key(q, &first.job_id))
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(old_exists, 0, "the previous pending job is gone");
    let wait: Vec<String> = redis::cmd("LRANGE")
        .arg(key(q, "wait"))
        .arg(0)
        .arg(-1)
        .query_async(&mut s.redis)
        .await
        .unwrap();
    let delayed: Vec<String> = redis::cmd("ZRANGE")
        .arg(key(q, "delayed"))
        .arg(0)
        .arg(-1)
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(
        wait.len() + delayed.len(),
        1,
        "exactly one pending job: {wait:?} {delayed:?}"
    );
    let sched = hash(&mut s.redis, key(q, "repeat:tick")).await;
    assert_eq!(sched["every"], "5000");
    assert_eq!(sched["data"], r#"{"v":2}"#);
    assert_eq!(sched["ic"], "7", "the iteration count survives an edit");

    // Switching strategy: every → pattern.
    upsert(
        &s,
        q,
        "tick",
        Schedule::Pattern {
            pattern: "*/5 * * * *".into(),
            tz: None,
        },
        "{}",
    )
    .await
    .expect("to pattern");
    let sched = hash(&mut s.redis, key(q, "repeat:tick")).await;
    assert_eq!(sched["pattern"], "*/5 * * * *");
    assert!(!sched.contains_key("every"));
    // No timezone given: stored as UTC, never left to the workers' local time.
    assert_eq!(sched["tz"], "UTC");

    assert_eq!(audit_count(&s, "upsertScheduler").await, 3);
    s.registry.shutdown_all().await;
    cleanup(&mut s.redis, q).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn pause_removes_the_pending_job_resume_recreates_it_remove_leaves_nothing() {
    let mut s = setup("env-local").await;
    let q = "lifecycle";
    seed_meta(&mut s.redis, q).await;
    let baseline = keys_of(&mut s.redis, q).await;

    let saved = upsert(
        &s,
        q,
        "hourly",
        Schedule::Pattern {
            pattern: "0 * * * *".into(),
            tz: None,
        },
        r#"{"a":1}"#,
    )
    .await
    .expect("create");

    // Pause: still listed (flagged), nothing left to fire.
    let paused = pause_scheduler_inner(
        action(&s, q, "hourly"),
        &common::backends(s.db.pool(), &s.kc, &s.registry),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .expect("pause");
    assert!(paused.paused && !paused.already_in_state);
    let pending: i64 = redis::cmd("EXISTS")
        .arg(key(q, &saved.job_id))
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(
        pending, 0,
        "the pending job is removed: nothing fires while paused"
    );
    let delayed: i64 = redis::cmd("ZCARD")
        .arg(key(q, "delayed"))
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(delayed, 0);
    let listed = list_cron_jobs_inner(
        ListCronJobsArgs {
            connection_id: s.conn_id.clone(),
            queue_id: qref(q).id().into(),
        },
        &common::backends(s.db.pool(), &s.kc, &s.registry),
    )
    .await
    .expect("list");
    assert_eq!(
        listed.len(),
        1,
        "a paused scheduler stays visible; it could not be resumed otherwise"
    );
    assert!(listed[0].paused);
    let again = pause_scheduler_inner(
        action(&s, q, "hourly"),
        &common::backends(s.db.pool(), &s.kc, &s.registry),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .expect("pause again");
    assert!(again.already_in_state);

    // Resume: a pending job exists again, the flag is gone, settings intact.
    let resumed = resume_scheduler_inner(
        action(&s, q, "hourly"),
        &common::backends(s.db.pool(), &s.kc, &s.registry),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .expect("resume");
    assert!(!resumed.paused && !resumed.already_in_state);
    let sched = hash(&mut s.redis, key(q, "repeat:hourly")).await;
    assert!(!sched.contains_key("paused"));
    assert_eq!(sched["pattern"], "0 * * * *");
    assert_eq!(sched["data"], r#"{"a":1}"#);
    let delayed: Vec<String> = redis::cmd("ZRANGE")
        .arg(key(q, "delayed"))
        .arg(0)
        .arg(-1)
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(delayed.len(), 1);
    assert!(delayed[0].starts_with("repeat:hourly:"));

    // Remove: every key the scheduler brought is gone.
    remove_scheduler_inner(
        action(&s, q, "hourly"),
        &common::backends(s.db.pool(), &s.kc, &s.registry),
        Caller::User,
    )
    .await
    .expect("remove");
    let left: Vec<String> = keys_of(&mut s.redis, q)
        .await
        .into_iter()
        .filter(|k| !baseline.contains(k))
        .collect();
    let expected: Vec<String> = ["events", "id", "marker"]
        .iter()
        .map(|k| key(q, k))
        .collect();
    assert_eq!(left, expected, "only BullMQ's bookkeeping remains");

    let err = remove_scheduler_inner(
        action(&s, q, "hourly"),
        &common::backends(s.db.pool(), &s.kc, &s.registry),
        Caller::User,
    )
    .await
    .expect_err("gone");
    assert!(
        matches!(err, RanchError::SchedulerNotFound { .. }),
        "{err:?}"
    );
    assert_eq!(
        audit_count(&s, "removeScheduler").await,
        1,
        "a miss is not audited"
    );

    s.registry.shutdown_all().await;
    cleanup(&mut s.redis, q).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn refuses_invalid_schedulers_unknown_queues_and_locked_connections() {
    let mut s = setup("env-local").await;
    let q = "refusals";
    seed_meta(&mut s.redis, q).await;
    let before = keys_of(&mut s.redis, q).await;

    for (schedule, expected) in [
        (
            Schedule::Pattern {
                pattern: "0 3 L * *".into(),
                tz: None,
            },
            "not supported",
        ),
        (
            Schedule::Pattern {
                pattern: "0 3 * * *".into(),
                tz: Some("Mars/Olympus".into()),
            },
            "Unknown timezone",
        ),
        (Schedule::Every { every_ms: 10 }, "at least 1000 ms"),
    ] {
        let err = upsert(&s, q, "x", schedule, "{}")
            .await
            .expect_err(expected);
        assert!(err.to_string().contains(expected), "{err}");
    }
    let err = upsert(&s, q, "a:b", Schedule::Every { every_ms: 5_000 }, "{}")
        .await
        .expect_err("id");
    assert!(err.to_string().contains("cannot contain ':'"), "{err}");
    let err = upsert(&s, "typo", "x", Schedule::Every { every_ms: 5_000 }, "{}")
        .await
        .expect_err("queue");
    assert!(err.to_string().contains("does not exist"), "{err}");
    assert_eq!(keys_of(&mut s.redis, q).await, before, "nothing written");
    assert_eq!(
        audit_count(&s, "upsertScheduler").await,
        0,
        "nothing was attempted"
    );

    let mut locked = setup("env-production").await;
    let err = upsert(&locked, q, "x", Schedule::Every { every_ms: 5_000 }, "{}")
        .await
        .expect_err("locked");
    assert!(
        matches!(err, RanchError::ConnectionLocked { .. }),
        "{err:?}"
    );
    let err = remove_scheduler_inner(
        action(&locked, q, "x"),
        &common::backends(locked.db.pool(), &locked.kc, &locked.registry),
        Caller::User,
    )
    .await
    .expect_err("locked");
    assert!(
        matches!(err, RanchError::ConnectionLocked { .. }),
        "{err:?}"
    );
    assert_eq!(keys_of(&mut locked.redis, q).await, before);

    s.registry.shutdown_all().await;
    locked.registry.shutdown_all().await;
    cleanup(&mut s.redis, q).await;
}
