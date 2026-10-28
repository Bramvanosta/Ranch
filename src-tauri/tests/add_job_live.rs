//! Live tests for `add_job`: what lands in Redis, the
//! paused / concurrency rules, and the safety rails. The proof that real
//! BullMQ workers accept it lives in `add_job_compat_live.rs`.
//!
//!   docker compose --profile redis up -d
//!   RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test add_job_live -- --include-ignored
//!
//! Self-seeded under the `ranch94` prefix; every test cleans up its queue.

// Same decision as the library (`src/lib.rs`): `RanchError` is wide and typed by design.
#![allow(clippy::result_large_err)]

use ranch_lib::adapters::bullmq_redis::bullmq_schema::QueueRef;
use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::audit::{read_audit_log, ListAuditLogEntriesArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::jobs::NewJob;
use ranch_lib::commands::jobs::{add_job_inner, AddJobArgs};
use ranch_lib::domain::audit::AuditOutcome;
use ranch_lib::domain::errors::RanchError;
use std::collections::HashMap;
use std::sync::Arc;

mod common;
use common::{redis_target, repo};

const PREFIX: &str = "ranch94";

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

/// A queue exists once BullMQ wrote its `meta`.
async fn seed_queue(
    redis: &mut redis::aio::MultiplexedConnection,
    queue: &str,
    meta: &[(&str, &str)],
) {
    cleanup(redis, queue).await;
    let mut cmd = redis::cmd("HSET");
    cmd.arg(format!("{PREFIX}:{queue}:meta"))
        .arg("version")
        .arg("bullmq:5.81.5");
    for (k, v) in meta {
        cmd.arg(k).arg(v);
    }
    let _: () = cmd.query_async(redis).await.unwrap();
}

fn job(name: &str) -> NewJob {
    NewJob {
        name: name.into(),
        data: r#"{"to":"a@b.c"}"#.into(),
        delay_ms: 0,
        attempts: None,
        priority: 0,
        lifo: false,
        backoff: None,
        job_id: None,
        remove_on_complete: false,
        remove_on_fail: false,
    }
}

async fn add(
    s: &Setup,
    queue: &str,
    job: NewJob,
) -> Result<ranch_lib::commands::jobs::AddJobResult, RanchError> {
    add_job_inner(
        AddJobArgs {
            connection_id: s.conn_id.clone(),
            queue_id: QueueRef {
                prefix: PREFIX.into(),
                name: queue.into(),
            }
            .id()
            .into(),
            job,
        },
        &common::backends(s.db.pool(), &s.kc, &s.registry),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
}

async fn audit_rows(s: &Setup) -> Vec<ranch_lib::domain::audit::AuditLogEntry> {
    read_audit_log(
        ListAuditLogEntriesArgs {
            from: None,
            to: None,
            connection_id: Some(s.conn_id.clone()),
            action: Some("addJob".into()),
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
async fn writes_the_hash_the_list_the_marker_and_the_events_bullmq_expects() {
    let mut s = setup("env-local").await;
    let queue = "plain";
    seed_queue(&mut s.redis, queue, &[]).await;

    let out = add(
        &s,
        queue,
        NewJob {
            attempts: Some(3),
            ..job("send-email")
        },
    )
    .await
    .expect("add_job");
    assert_eq!(out.job_id, "1");
    assert_eq!(out.state.as_str(), "waiting");

    let hash: HashMap<String, String> = redis::cmd("HGETALL")
        .arg(format!("{PREFIX}:{queue}:1"))
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(hash["name"], "send-email");
    assert_eq!(hash["data"], r#"{"to":"a@b.c"}"#);
    assert_eq!(hash["opts"], r#"{"attempts":3}"#);
    assert_eq!(hash["delay"], "0");
    assert_eq!(hash["priority"], "0");
    let ts: i64 = hash["timestamp"].parse().expect("integer ms, not 1.78e12");
    assert!((chrono::Utc::now().timestamp_millis() - ts).abs() < 5_000);
    assert_eq!(hash.len(), 6, "nothing BullMQ does not write: {hash:?}");

    let wait: Vec<String> = redis::cmd("LRANGE")
        .arg(format!("{PREFIX}:{queue}:wait"))
        .arg(0)
        .arg(-1)
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(wait, vec!["1"]);
    let marker: Vec<(String, f64)> = redis::cmd("ZRANGE")
        .arg(format!("{PREFIX}:{queue}:marker"))
        .arg(0)
        .arg(-1)
        .arg("WITHSCORES")
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(
        marker,
        vec![("0".to_string(), 0.0)],
        "idle workers are woken"
    );
    let events: i64 = redis::cmd("XLEN")
        .arg(format!("{PREFIX}:{queue}:events"))
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(events, 2, "`added` + `waiting`");
    let max_len: String = redis::cmd("HGET")
        .arg(format!("{PREFIX}:{queue}:meta"))
        .arg("opts.maxLenEvents")
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(max_len, "10000");

    let rows = audit_rows(&s).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, AuditOutcome::Success);

    s.registry.shutdown_all().await;
    cleanup(&mut s.redis, queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn a_paused_queue_gets_the_job_in_its_paused_list_without_waking_workers() {
    let mut s = setup("env-local").await;
    let queue = "paused";
    seed_queue(&mut s.redis, queue, &[("paused", "1")]).await;

    let out = add(&s, queue, job("x")).await.expect("add_job");
    assert_eq!(out.state.as_str(), "paused");
    let prio = add(
        &s,
        queue,
        NewJob {
            priority: 3,
            ..job("y")
        },
    )
    .await
    .expect("add_job");
    assert_eq!(prio.state.as_str(), "prioritized");

    let paused_len: i64 = redis::cmd("LLEN")
        .arg(format!("{PREFIX}:{queue}:paused"))
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(paused_len, 1);
    // Putting it in `wait` would lose it: resume RENAMEs `paused` over `wait`.
    for key in ["wait", "marker"] {
        let exists: i64 = redis::cmd("EXISTS")
            .arg(format!("{PREFIX}:{queue}:{key}"))
            .query_async(&mut s.redis)
            .await
            .unwrap();
        assert_eq!(exists, 0, "{key} must not exist on a paused queue");
    }

    s.registry.shutdown_all().await;
    cleanup(&mut s.redis, queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn delayed_jobs_keep_their_order_within_a_millisecond_and_arm_the_marker() {
    let mut s = setup("env-local").await;
    let queue = "delayed";
    seed_queue(&mut s.redis, queue, &[]).await;

    for _ in 0..3 {
        let out = add(
            &s,
            queue,
            NewJob {
                delay_ms: 60_000,
                ..job("later")
            },
        )
        .await
        .expect("add_job");
        assert_eq!(out.state.as_str(), "delayed");
    }
    let delayed: Vec<(String, f64)> = redis::cmd("ZRANGE")
        .arg(format!("{PREFIX}:{queue}:delayed"))
        .arg(0)
        .arg(-1)
        .arg("WITHSCORES")
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(
        delayed
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["1", "2", "3"]
    );
    let due_ms = (delayed[0].1 / 4096.0) as i64;
    let in_ms = due_ms - chrono::Utc::now().timestamp_millis();
    assert!((55_000..=60_000).contains(&in_ms), "due in {in_ms} ms");
    assert!(
        delayed[0].1 < delayed[1].1 && delayed[1].1 < delayed[2].1,
        "strictly increasing"
    );

    // Marker member "1" = "a delayed job is due at <score>".
    let marker: Vec<(String, f64)> = redis::cmd("ZRANGE")
        .arg(format!("{PREFIX}:{queue}:marker"))
        .arg(0)
        .arg(-1)
        .arg("WITHSCORES")
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(marker.len(), 1);
    assert_eq!(marker[0].0, "1");
    assert_eq!(marker[0].1 as i64, due_ms);

    s.registry.shutdown_all().await;
    cleanup(&mut s.redis, queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn never_overwrites_a_job_that_already_sits_on_the_next_id() {
    let mut s = setup("env-local").await;
    let queue = "collision";
    seed_queue(&mut s.redis, queue, &[]).await;
    // A job added by the app with the custom id "1".
    let _: () = redis::cmd("HSET")
        .arg(format!("{PREFIX}:{queue}:1"))
        .arg("name")
        .arg("theirs")
        .query_async(&mut s.redis)
        .await
        .unwrap();

    let out = add(&s, queue, job("ours")).await.expect("add_job");
    assert_eq!(out.job_id, "2");
    let theirs: String = redis::cmd("HGET")
        .arg(format!("{PREFIX}:{queue}:1"))
        .arg("name")
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(theirs, "theirs");

    s.registry.shutdown_all().await;
    cleanup(&mut s.redis, queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn refuses_a_queue_that_does_not_exist_and_an_invalid_job_without_touching_redis() {
    let mut s = setup("env-local").await;
    seed_queue(&mut s.redis, "real", &[]).await;

    let err = add(&s, "typo", job("x")).await.expect_err("unknown queue");
    assert!(err.to_string().contains("does not exist"), "{err}");
    let err = add(
        &s,
        "real",
        NewJob {
            data: "{oops".into(),
            ..job("x")
        },
    )
    .await
    .expect_err("invalid JSON");
    assert!(err.to_string().contains("not valid JSON"), "{err}");

    let keys: Vec<String> = redis::cmd("KEYS")
        .arg(format!("{PREFIX}:*"))
        .query_async(&mut s.redis)
        .await
        .unwrap();
    let mut keys: Vec<&str> = keys
        .iter()
        .map(String::as_str)
        .filter(|k| k.contains(":typo:") || k.contains(":real:"))
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [format!("{PREFIX}:real:meta")],
        "only the seeded meta"
    );
    assert!(audit_rows(&s).await.is_empty(), "nothing was attempted");

    s.registry.shutdown_all().await;
    cleanup(&mut s.redis, "real").await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn a_locked_connection_refuses_before_redis() {
    let mut s = setup("env-production").await;
    let queue = "locked";
    seed_queue(&mut s.redis, queue, &[]).await;

    let err = add(&s, queue, job("x")).await.expect_err("locked");
    assert!(
        matches!(err, RanchError::ConnectionLocked { .. }),
        "{err:?}"
    );
    let id_exists: i64 = redis::cmd("EXISTS")
        .arg(format!("{PREFIX}:{queue}:id"))
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(id_exists, 0, "not even the id counter moved");

    s.registry.shutdown_all().await;
    cleanup(&mut s.redis, queue).await;
}

/// Found while building Add job: `is_paused` looked for the `paused` *list*,
/// which only exists if jobs were waiting when the queue was paused (an
/// empty paused queue read as running), and the counters ignored the
/// `paused` and `prioritized` containers, so jobs sitting there were
/// invisible.
#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn queue_counters_see_paused_and_prioritized_jobs_and_an_empty_paused_queue() {
    let mut s = setup("env-local").await;
    let queue = "counters";
    seed_queue(&mut s.redis, queue, &[("paused", "1")]).await;

    let find = |queues: Vec<ranch_lib::commands::queues::Queue>| {
        queues
            .into_iter()
            .find(|q| q.id.as_str() == format!("{PREFIX}:{queue}"))
            .expect("queue discovered")
    };
    let backends = common::backends(s.db.pool(), &s.kc, &s.registry);
    let list = || ranch_lib::commands::queues::list_queues_inner(&s.conn_id, &backends);

    let empty = find(list().await.expect("list_queues"));
    assert!(
        empty.is_paused,
        "paused by meta flag, without any `paused` list"
    );
    assert_eq!(empty.counts.get("paused"), 0);

    add(&s, queue, job("parked")).await.expect("add_job");
    add(
        &s,
        queue,
        NewJob {
            priority: 2,
            ..job("vip")
        },
    )
    .await
    .expect("add_job");
    let after = find(list().await.expect("list_queues"));
    assert!(after.is_paused);
    assert_eq!(after.counts.get("waiting"), 0);
    assert_eq!(after.counts.get("paused"), 1);
    assert_eq!(after.counts.get("prioritized"), 1);

    s.registry.shutdown_all().await;
    cleanup(&mut s.redis, queue).await;
}

// A custom job ID is the job's key; taken, nothing is written.
#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn a_custom_id_is_used_once_and_a_taken_one_adds_nothing() {
    let mut s = setup("env-local").await;
    let queue = "custom-id";
    seed_queue(&mut s.redis, queue, &[]).await;

    let first = add(
        &s,
        queue,
        NewJob {
            job_id: Some("order-1042".into()),
            remove_on_complete: true,
            ..job("send-email")
        },
    )
    .await
    .expect("add_job");
    assert_eq!(first.job_id, "order-1042");
    let opts: String = redis::cmd("HGET")
        .arg(format!("{PREFIX}:{queue}:order-1042"))
        .arg("opts")
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(opts, r#"{"jobId":"order-1042","removeOnComplete":true}"#);
    let counter: Option<i64> = redis::cmd("GET")
        .arg(format!("{PREFIX}:{queue}:id"))
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(counter, None, "a custom id leaves the counter alone");

    let again = add(
        &s,
        queue,
        NewJob {
            job_id: Some("order-1042".into()),
            ..job("other")
        },
    )
    .await
    .expect_err("taken");
    assert!(again.to_string().contains("already exists"), "{again}");
    let name: String = redis::cmd("HGET")
        .arg(format!("{PREFIX}:{queue}:order-1042"))
        .arg("name")
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(name, "send-email", "the first job is untouched");
    let wait: Vec<String> = redis::cmd("LRANGE")
        .arg(format!("{PREFIX}:{queue}:wait"))
        .arg(0)
        .arg(-1)
        .query_async(&mut s.redis)
        .await
        .unwrap();
    assert_eq!(wait, vec!["order-1042"]);

    cleanup(&mut s.redis, queue).await;
}
