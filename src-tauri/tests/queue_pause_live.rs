//! Live integration tests for pause/resume queue + scheduler.
//!
//! Ignored unless run with `--include-ignored` and `RANCH_LOCAL_REDIS=1`.
//!
//! ```bash
//! docker compose --profile redis up -d
//! RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 \
//!   cargo test --test queue_pause_live -- --include-ignored --nocapture
//! ```

use ranch_lib::adapters::bullmq_redis::bullmq_schema::QueueRef;
use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::commands::audit::{read_audit_log, ListAuditLogEntriesArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::queues::{
    pause_queue_inner, pause_scheduler_inner, resume_queue_inner, resume_scheduler_inner,
    QueueActionArgs, SchedulerActionArgs,
};
use ranch_lib::domain::audit::AuditOutcome;
use ranch_lib::domain::errors::RanchError;
use tempfile::tempdir;

mod common;
use common::{redis_target, repo};

async fn fresh_setup(
    env_color: &str,
) -> (
    tempfile::TempDir,
    Db,
    std::sync::Arc<dyn ranch_lib::application::ports::SecretStore>,
    ConnectionRegistry,
    String,
) {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());
    let registry = ConnectionRegistry::new();
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
    (dir, db, kc, registry, conn.id)
}

/// Seed a minimal BullMQ v5 queue with a `:wait` list + the `:meta` hash.
async fn seed_queue_with_wait(
    conn: &mut redis::aio::MultiplexedConnection,
    queue: &str,
    job_ids: &[&str],
) {
    let meta_key = format!("bull:{queue}:meta");
    let wait_key = format!("bull:{queue}:wait");
    let paused_key = format!("bull:{queue}:paused");

    // Clean slate
    for k in [&meta_key, &wait_key, &paused_key] {
        let _: i64 = redis::cmd("DEL").arg(k).query_async(conn).await.unwrap();
    }

    redis::cmd("HSET")
        .arg(&meta_key)
        .arg("version")
        .arg("bullmq:5.0.0")
        .exec_async(conn)
        .await
        .unwrap();
    if !job_ids.is_empty() {
        let mut lpush = redis::cmd("LPUSH");
        lpush.arg(&wait_key);
        for id in job_ids {
            lpush.arg(*id);
        }
        lpush.exec_async(conn).await.unwrap();
    }
}

async fn seed_scheduler(
    conn: &mut redis::aio::MultiplexedConnection,
    queue: &str,
    scheduler_key: &str,
    next_run_ms: i64,
) {
    let repeat_key = format!("bull:{queue}:repeat");
    let scheduler_hash_key = format!("bull:{queue}:repeat:{scheduler_key}");
    // Ensure :meta exists so schema detection works.
    let meta_key = format!("bull:{queue}:meta");
    redis::cmd("HSET")
        .arg(&meta_key)
        .arg("version")
        .arg("bullmq:5.0.0")
        .exec_async(conn)
        .await
        .unwrap();
    // ZADD the entry.
    redis::cmd("ZADD")
        .arg(&repeat_key)
        .arg(next_run_ms)
        .arg(scheduler_key)
        .exec_async(conn)
        .await
        .unwrap();
    // The per-scheduler hash carries `name` + either `pattern` or `every`.
    let _: i64 = redis::cmd("DEL")
        .arg(&scheduler_hash_key)
        .query_async(conn)
        .await
        .unwrap();
    redis::cmd("HSET")
        .arg(&scheduler_hash_key)
        .arg("name")
        .arg("test-scheduler")
        .arg("every")
        .arg("60000")
        .arg("ic")
        .arg("0")
        .exec_async(conn)
        .await
        .unwrap();
}

async fn cleanup_queue(conn: &mut redis::aio::MultiplexedConnection, queue: &str) {
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg(format!("bull:{queue}:*"))
        .query_async(conn)
        .await
        .unwrap_or_default();
    for k in keys {
        let _: i64 = redis::cmd("DEL").arg(&k).query_async(conn).await.unwrap();
    }
}

async fn audit_rows_for(
    db: &Db,
    connection_id: &str,
    action: &str,
) -> Vec<ranch_lib::domain::audit::AuditLogEntry> {
    read_audit_log(
        ListAuditLogEntriesArgs {
            from: None,
            to: None,
            connection_id: Some(connection_id.into()),
            action: Some(action.into()),
            source: None,
            offset: 0,
            limit: 100,
        },
        db.pool(),
    )
    .await
    .unwrap()
}

fn qref(queue: &str) -> QueueRef {
    QueueRef {
        prefix: "bull".into(),
        name: queue.into(),
    }
}

// ─── Queue pause/resume ─────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn pause_queue_renames_wait_to_paused_and_sets_meta_flag() {
    let (_d, db, kc, reg, conn_id) = fresh_setup("env-local").await;
    let queue = "ranch-test-pause-queue-1";
    let mut rconn = redis_target::connect().await;
    cleanup_queue(&mut rconn, queue).await;
    seed_queue_with_wait(&mut rconn, queue, &["j1", "j2", "j3"]).await;

    let result = pause_queue_inner(
        QueueActionArgs {
            connection_id: conn_id.clone(),
            queue_id: qref(queue).id().into(),
        },
        &common::backends(db.pool(), &kc, &reg),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .expect("pause_queue");

    assert!(result.is_paused);
    assert!(!result.already_in_state);

    // Verify Redis state: :wait gone, :paused has 3 entries, :meta paused = 1.
    let wait_exists: i64 = redis::cmd("EXISTS")
        .arg(format!("bull:{queue}:wait"))
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(wait_exists, 0);
    let paused_len: i64 = redis::cmd("LLEN")
        .arg(format!("bull:{queue}:paused"))
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(paused_len, 3);
    let meta_paused: Option<String> = redis::cmd("HGET")
        .arg(format!("bull:{queue}:meta"))
        .arg("paused")
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(meta_paused.as_deref(), Some("1"));

    // Audit row.
    let rows = audit_rows_for(&db, &conn_id, "pauseQueue").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, AuditOutcome::Success);
    assert!(rows[0].outcome_reason.is_none());

    cleanup_queue(&mut rconn, queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn resume_queue_renames_paused_to_wait_and_clears_meta_flag() {
    let (_d, db, kc, reg, conn_id) = fresh_setup("env-local").await;
    let queue = "ranch-test-resume-queue-1";
    let mut rconn = redis_target::connect().await;
    cleanup_queue(&mut rconn, queue).await;
    seed_queue_with_wait(&mut rconn, queue, &["j1", "j2"]).await;

    // Pause first.
    pause_queue_inner(
        QueueActionArgs {
            connection_id: conn_id.clone(),
            queue_id: qref(queue).id().into(),
        },
        &common::backends(db.pool(), &kc, &reg),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .unwrap();

    let result = resume_queue_inner(
        QueueActionArgs {
            connection_id: conn_id.clone(),
            queue_id: qref(queue).id().into(),
        },
        &common::backends(db.pool(), &kc, &reg),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .expect("resume_queue");

    assert!(!result.is_paused);
    assert!(!result.already_in_state);

    let paused_exists: i64 = redis::cmd("EXISTS")
        .arg(format!("bull:{queue}:paused"))
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(paused_exists, 0);
    let wait_len: i64 = redis::cmd("LLEN")
        .arg(format!("bull:{queue}:wait"))
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(wait_len, 2);
    let meta_paused: Option<String> = redis::cmd("HGET")
        .arg(format!("bull:{queue}:meta"))
        .arg("paused")
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(
        meta_paused, None,
        "BullMQ reads any `paused` field as paused"
    );

    let rows = audit_rows_for(&db, &conn_id, "resumeQueue").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, AuditOutcome::Success);

    cleanup_queue(&mut rconn, queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn pause_when_already_paused_is_idempotent_and_audits() {
    let (_d, db, kc, reg, conn_id) = fresh_setup("env-local").await;
    let queue = "ranch-test-pause-idempotent";
    let mut rconn = redis_target::connect().await;
    cleanup_queue(&mut rconn, queue).await;
    seed_queue_with_wait(&mut rconn, queue, &["j1"]).await;

    // First pause.
    pause_queue_inner(
        QueueActionArgs {
            connection_id: conn_id.clone(),
            queue_id: qref(queue).id().into(),
        },
        &common::backends(db.pool(), &kc, &reg),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .unwrap();

    // Second pause, idempotent.
    let result = pause_queue_inner(
        QueueActionArgs {
            connection_id: conn_id.clone(),
            queue_id: qref(queue).id().into(),
        },
        &common::backends(db.pool(), &kc, &reg),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .expect("pause_queue idempotent");

    assert!(result.is_paused);
    assert!(result.already_in_state, "second pause should be a no-op");

    // 2 audit rows total; second has the "already paused" reason.
    let rows = audit_rows_for(&db, &conn_id, "pauseQueue").await;
    assert_eq!(rows.len(), 2);
    // Audit log is newest-first; row[0] is the latest call.
    assert_eq!(rows[0].outcome_reason.as_deref(), Some("already paused"));

    cleanup_queue(&mut rconn, queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn resume_when_already_running_is_idempotent_and_audits() {
    let (_d, db, kc, reg, conn_id) = fresh_setup("env-local").await;
    let queue = "ranch-test-resume-idempotent";
    let mut rconn = redis_target::connect().await;
    cleanup_queue(&mut rconn, queue).await;
    seed_queue_with_wait(&mut rconn, queue, &["j1"]).await;

    // Queue is running by default; resume should be a no-op.
    let result = resume_queue_inner(
        QueueActionArgs {
            connection_id: conn_id.clone(),
            queue_id: qref(queue).id().into(),
        },
        &common::backends(db.pool(), &kc, &reg),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .expect("resume_queue idempotent");

    assert!(!result.is_paused);
    assert!(result.already_in_state);
    // A running queue keeps no `paused` field: BullMQ would read any
    // value, "0" included, as paused.
    let flag: i64 = redis::cmd("HEXISTS")
        .arg(format!("bull:{queue}:meta"))
        .arg("paused")
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(flag, 0);

    let rows = audit_rows_for(&db, &conn_id, "resumeQueue").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome_reason.as_deref(), Some("already running"));

    cleanup_queue(&mut rconn, queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn pause_queue_on_locked_connection_returns_connection_locked_and_no_redis_mutation() {
    let (_d, db, kc, reg, conn_id) = fresh_setup("env-production").await;
    let queue = "ranch-test-pause-locked";
    let mut rconn = redis_target::connect().await;
    cleanup_queue(&mut rconn, queue).await;
    seed_queue_with_wait(&mut rconn, queue, &["j1", "j2"]).await;

    let err = pause_queue_inner(
        QueueActionArgs {
            connection_id: conn_id.clone(),
            queue_id: qref(queue).id().into(),
        },
        &common::backends(db.pool(), &kc, &reg),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .expect_err("pause_queue should reject on locked connection");

    assert!(matches!(err, RanchError::ConnectionLocked { .. }));

    // No Redis mutation: :wait still has 2 entries, no :paused key.
    let wait_len: i64 = redis::cmd("LLEN")
        .arg(format!("bull:{queue}:wait"))
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(wait_len, 2);
    let paused_exists: i64 = redis::cmd("EXISTS")
        .arg(format!("bull:{queue}:paused"))
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(paused_exists, 0);

    // One failure row: the log shows the refused attempt.
    let rows = audit_rows_for(&db, &conn_id, "pauseQueue").await;
    assert_eq!(rows.len(), 1);

    cleanup_queue(&mut rconn, queue).await;
}

// ─── Scheduler pause/resume ─────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn pause_scheduler_keeps_it_listed_and_sets_paused_flag() {
    let (_d, db, kc, reg, conn_id) = fresh_setup("env-local").await;
    let queue = "ranch-test-pause-scheduler-1";
    let scheduler_key = "every-15m";
    let next_run_ms: i64 = 1_778_487_835_788;
    let mut rconn = redis_target::connect().await;
    cleanup_queue(&mut rconn, queue).await;
    seed_scheduler(&mut rconn, queue, scheduler_key, next_run_ms).await;

    let result = pause_scheduler_inner(
        SchedulerActionArgs {
            connection_id: conn_id.clone(),
            queue_id: qref(queue).id().into(),
            scheduler_key: scheduler_key.into(),
        },
        &common::backends(db.pool(), &kc, &reg),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .expect("pause_scheduler");

    assert!(result.paused);
    assert!(!result.already_in_state);

    // The scheduler STAYS in `:repeat`: Ranch's own list (and
    // BullMQ's) reads that set, so a scheduler removed from it could never be
    // resumed from the UI. What stops it is the removal of its pending job,
    // covered in `scheduler_live.rs`.
    let zscore: Option<f64> = redis::cmd("ZSCORE")
        .arg(format!("bull:{queue}:repeat"))
        .arg(scheduler_key)
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(zscore, Some(next_run_ms as f64), "still listed");

    // Hash flagged.
    let paused: Option<String> = redis::cmd("HGET")
        .arg(format!("bull:{queue}:repeat:{scheduler_key}"))
        .arg("paused")
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(paused.as_deref(), Some("1"));

    let rows = audit_rows_for(&db, &conn_id, "pauseScheduler").await;
    assert_eq!(rows.len(), 1);

    cleanup_queue(&mut rconn, queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn resume_scheduler_zadds_back_and_clears_flag() {
    let (_d, db, kc, reg, conn_id) = fresh_setup("env-local").await;
    let queue = "ranch-test-resume-scheduler-1";
    let scheduler_key = "every-15m";
    let next_run_ms: i64 = 1_778_487_835_788;
    let mut rconn = redis_target::connect().await;
    cleanup_queue(&mut rconn, queue).await;
    seed_scheduler(&mut rconn, queue, scheduler_key, next_run_ms).await;

    // Pause first.
    pause_scheduler_inner(
        SchedulerActionArgs {
            connection_id: conn_id.clone(),
            queue_id: qref(queue).id().into(),
            scheduler_key: scheduler_key.into(),
        },
        &common::backends(db.pool(), &kc, &reg),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .unwrap();

    let result = resume_scheduler_inner(
        SchedulerActionArgs {
            connection_id: conn_id.clone(),
            queue_id: qref(queue).id().into(),
            scheduler_key: scheduler_key.into(),
        },
        &common::backends(db.pool(), &kc, &reg),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .expect("resume_scheduler");

    assert!(!result.paused);

    // Re-armed in :repeat with a fresh timestamp (≥ next_run_ms is good
    // enough; we use Utc::now() which is > the seeded 2026 epoch).
    let zscore: Option<f64> = redis::cmd("ZSCORE")
        .arg(format!("bull:{queue}:repeat"))
        .arg(scheduler_key)
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert!(zscore.is_some());

    // Flags cleared.
    let paused: Option<String> = redis::cmd("HGET")
        .arg(format!("bull:{queue}:repeat:{scheduler_key}"))
        .arg("paused")
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert!(paused.is_none(), "paused flag should be HDEL'd");

    let rows = audit_rows_for(&db, &conn_id, "resumeScheduler").await;
    assert_eq!(rows.len(), 1);

    cleanup_queue(&mut rconn, queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn pause_scheduler_unknown_key_returns_scheduler_not_found_and_no_audit() {
    let (_d, db, kc, reg, conn_id) = fresh_setup("env-local").await;
    let queue = "ranch-test-pause-scheduler-unknown";
    let mut rconn = redis_target::connect().await;
    cleanup_queue(&mut rconn, queue).await;
    // Seed only the :meta hash so schema detection passes; no scheduler.
    let meta_key = format!("bull:{queue}:meta");
    redis::cmd("HSET")
        .arg(&meta_key)
        .arg("version")
        .arg("bullmq:5.0.0")
        .exec_async(&mut rconn)
        .await
        .unwrap();

    let err = pause_scheduler_inner(
        SchedulerActionArgs {
            connection_id: conn_id.clone(),
            queue_id: qref(queue).id().into(),
            scheduler_key: "does-not-exist".into(),
        },
        &common::backends(db.pool(), &kc, &reg),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .expect_err("pause_scheduler should reject unknown key");

    assert!(matches!(err, RanchError::SchedulerNotFound { .. }));

    // Precondition violations don't audit.
    let rows = audit_rows_for(&db, &conn_id, "pauseScheduler").await;
    assert_eq!(rows.len(), 0);

    cleanup_queue(&mut rconn, queue).await;
}
