//! Live integration tests for single-job actions
//! (retry / remove / promote).
//!
//! Skips unless `RANCH_LOCAL_REDIS=1` is set. Each test seeds its own
//! isolated job into a unique queue (`ranch-test-action-{action}-{n}`)
//! to avoid cross-test interference with the BullMQ seed used by the
//! other live tests.
//!
//! ```bash
//! docker compose --profile redis up -d
//! RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 \
//!   cargo test --test job_actions_live -- --nocapture
//! ```
//!
//! What each test asserts:
//!   - The action's Redis effect (state transition / hash deletion).
//!   - An audit entry is written via `core::audit::record`.
//!   - State-precondition violations return `InvalidJobState` and do
//!     NOT write audit entries.
//!   - Lock rejection returns `ConnectionLocked` BEFORE any Redis op.

use ranch_lib::commands::audit::{list_audit_log_entries_inner, ListAuditLogEntriesArgs};
use ranch_lib::commands::jobs::{JobActionArgs, JobActionResult};
use ranch_lib::core::bullmq_schema::{JobState, QueueRef};
use ranch_lib::core::connection_actor::{ConnectionConfig, ConnectionRegistry};
use ranch_lib::core::db::Db;
use ranch_lib::core::errors::RanchError;
use ranch_lib::core::safety_lock::ReadOnlyState;
use tempfile::tempdir;

fn skip_if_no_local_redis() -> bool {
    if std::env::var("RANCH_LOCAL_REDIS").is_err() {
        eprintln!(
            "skipping: set RANCH_LOCAL_REDIS=1 (and optionally RANCH_REDIS_HOST/PORT) + run \
             `docker compose --profile redis up -d`"
        );
        return true;
    }
    false
}

fn redis_host() -> String {
    std::env::var("RANCH_REDIS_HOST").unwrap_or_else(|_| "127.0.0.1".into())
}

fn redis_port() -> u16 {
    std::env::var("RANCH_REDIS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6390)
}

async fn fresh_pool() -> (tempfile::TempDir, Db) {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    (dir, db)
}

async fn cfg_with(state_db: &Db, conn_id: &str, lock_state: ReadOnlyState) -> ConnectionConfig {
    ConnectionConfig {
        host: redis_host(),
        port: redis_port(),
        db: 0,
        username: None,
        password: None,
        tls: false,
        tls_custom_ca: None,
        ssh_tunnel: None,
        ssh_password: None,
        keychain: std::sync::Arc::new(ranch_lib::core::keychain::InMemoryKeychain::new()),
        initial_lock_state: lock_state,
        connection_id: conn_id.into(),
        connection_name: format!("Test · {conn_id}"),
        env_color: Some("env-local".into()),
        pool: state_db.pool().clone(),
    }
}

/// Open a fresh redis::Client + connection for seeding. We bypass the
/// actor here on purpose — the actor IS what we're testing.
async fn open_redis() -> redis::aio::MultiplexedConnection {
    let url = format!("redis://{}:{}", redis_host(), redis_port());
    let client = redis::Client::open(url).expect("redis::Client::open");
    client
        .get_multiplexed_async_connection()
        .await
        .expect("multiplexed conn")
}

/// Seed a minimal BullMQ v5 queue + a single job in the given state.
/// Writes the `:meta` hash (so the schema decoder picks v5), the job
/// hash, and adds the job id to the correct state container.
async fn seed_job(
    conn: &mut redis::aio::MultiplexedConnection,
    queue: &str,
    job_id: &str,
    state: &str,
) {
    let meta_key = format!("bull:{queue}:meta");
    let job_hash_key = format!("bull:{queue}:{job_id}");

    // meta: minimal v5 declaration.
    redis::cmd("DEL")
        .arg(&meta_key)
        .exec_async(conn)
        .await
        .unwrap();
    redis::cmd("HSET")
        .arg(&meta_key)
        .arg("version")
        .arg("bullmq:5.0.0")
        .exec_async(conn)
        .await
        .unwrap();

    // job hash: minimum fields the decoder accepts (name + data + opts).
    redis::cmd("DEL")
        .arg(&job_hash_key)
        .exec_async(conn)
        .await
        .unwrap();
    let mut hset = redis::cmd("HSET");
    hset.arg(&job_hash_key)
        .arg("name")
        .arg("test-job")
        .arg("data")
        .arg("{}")
        .arg("opts")
        .arg("{}")
        .arg("timestamp")
        .arg("1700000000000")
        .arg("attemptsMade")
        .arg("0");
    if state == "failed" {
        hset.arg("failedReason").arg("kaboom");
        hset.arg("stacktrace").arg("[]");
    }
    hset.exec_async(conn).await.unwrap();

    // Add to the state container.
    let container_key = match state {
        "waiting" => format!("bull:{queue}:wait"),
        "active" => format!("bull:{queue}:active"),
        "failed" => format!("bull:{queue}:failed"),
        "delayed" => format!("bull:{queue}:delayed"),
        other => panic!("unknown seed state: {other}"),
    };
    redis::cmd("DEL")
        .arg(&container_key)
        .exec_async(conn)
        .await
        .unwrap();

    match state {
        "waiting" | "active" => {
            redis::cmd("LPUSH")
                .arg(&container_key)
                .arg(job_id)
                .exec_async(conn)
                .await
                .unwrap();
        }
        "failed" | "delayed" => {
            redis::cmd("ZADD")
                .arg(&container_key)
                .arg(1700000000000_i64)
                .arg(job_id)
                .exec_async(conn)
                .await
                .unwrap();
        }
        _ => unreachable!(),
    }
}

async fn cleanup_queue(conn: &mut redis::aio::MultiplexedConnection, queue: &str) {
    // Best-effort: blow away the keys we touched. Don't fail the test on cleanup errors.
    let mut scan = redis::cmd("KEYS");
    scan.arg(format!("bull:{queue}:*"));
    let keys: Vec<String> = scan.query_async(conn).await.unwrap_or_default();
    for k in keys {
        let _ = redis::cmd("DEL").arg(&k).exec_async(conn).await;
    }
}

async fn audit_entries_for(db: &Db, connection_id: &str, action: &str) -> usize {
    list_audit_log_entries_inner(
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
    .len()
}

fn args_for(conn_id: &str, queue: &str, job_id: &str) -> JobActionArgs {
    JobActionArgs {
        connection_id: conn_id.into(),
        queue_ref: QueueRef {
            prefix: "bull".into(),
            name: queue.into(),
        },
        job_id: job_id.into(),
    }
}

// ─── Retry — happy path ──────────────────────────────────────────────────

#[tokio::test]
async fn retry_moves_failed_job_to_waiting_and_writes_audit() {
    if skip_if_no_local_redis() {
        return;
    }
    let (_d, db) = fresh_pool().await;
    let queue = "ranch-test-retry-1";
    let job_id = "retry-job-1";
    let conn_id = "conn-retry-1";

    let mut conn = open_redis().await;
    cleanup_queue(&mut conn, queue).await;
    seed_job(&mut conn, queue, job_id, "failed").await;

    let registry = ConnectionRegistry::new();
    let cfg = cfg_with(&db, conn_id, ReadOnlyState::Unlocked).await;
    let result: JobActionResult = registry
        .retry_job(
            conn_id,
            &cfg,
            args_for(conn_id, queue, job_id),
            ranch_lib::core::connection_actor::AuditAttribution::ui(),
        )
        .await
        .expect("actor")
        .expect("retry_job");

    assert_eq!(result.action, "retry");
    assert_eq!(result.new_state, Some(JobState::Waiting));

    // Failed ZSET should no longer contain the job; wait list should.
    let score: Option<f64> = redis::cmd("ZSCORE")
        .arg(format!("bull:{queue}:failed"))
        .arg(job_id)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(score.is_none(), "job should no longer be in failed");
    let pos: Option<i64> = redis::cmd("LPOS")
        .arg(format!("bull:{queue}:wait"))
        .arg(job_id)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(pos.is_some(), "job should be in wait list");

    assert_eq!(audit_entries_for(&db, conn_id, "retryJob").await, 1);

    cleanup_queue(&mut conn, queue).await;
}

// ─── Remove — happy path ─────────────────────────────────────────────────

#[tokio::test]
async fn remove_deletes_hash_and_writes_audit() {
    if skip_if_no_local_redis() {
        return;
    }
    let (_d, db) = fresh_pool().await;
    let queue = "ranch-test-remove-1";
    let job_id = "remove-job-1";
    let conn_id = "conn-remove-1";

    let mut conn = open_redis().await;
    cleanup_queue(&mut conn, queue).await;
    seed_job(&mut conn, queue, job_id, "failed").await;

    let registry = ConnectionRegistry::new();
    let cfg = cfg_with(&db, conn_id, ReadOnlyState::Unlocked).await;
    let result = registry
        .remove_job(
            conn_id,
            &cfg,
            args_for(conn_id, queue, job_id),
            ranch_lib::core::connection_actor::AuditAttribution::ui(),
        )
        .await
        .expect("actor")
        .expect("remove_job");

    assert_eq!(result.action, "remove");
    assert!(result.new_state.is_none());

    let exists: i64 = redis::cmd("EXISTS")
        .arg(format!("bull:{queue}:{job_id}"))
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(exists, 0, "job hash should be gone");

    assert_eq!(audit_entries_for(&db, conn_id, "removeJob").await, 1);

    cleanup_queue(&mut conn, queue).await;
}

// ─── Promote — happy path ────────────────────────────────────────────────

#[tokio::test]
async fn promote_moves_delayed_job_to_waiting_and_writes_audit() {
    if skip_if_no_local_redis() {
        return;
    }
    let (_d, db) = fresh_pool().await;
    let queue = "ranch-test-promote-1";
    let job_id = "promote-job-1";
    let conn_id = "conn-promote-1";

    let mut conn = open_redis().await;
    cleanup_queue(&mut conn, queue).await;
    seed_job(&mut conn, queue, job_id, "delayed").await;

    let registry = ConnectionRegistry::new();
    let cfg = cfg_with(&db, conn_id, ReadOnlyState::Unlocked).await;
    let result = registry
        .promote_job(
            conn_id,
            &cfg,
            args_for(conn_id, queue, job_id),
            ranch_lib::core::connection_actor::AuditAttribution::ui(),
        )
        .await
        .expect("actor")
        .expect("promote_job");

    assert_eq!(result.action, "promote");
    assert_eq!(result.new_state, Some(JobState::Waiting));

    let z: Option<f64> = redis::cmd("ZSCORE")
        .arg(format!("bull:{queue}:delayed"))
        .arg(job_id)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(z.is_none(), "job should no longer be in delayed");
    let pos: Option<i64> = redis::cmd("LPOS")
        .arg(format!("bull:{queue}:wait"))
        .arg(job_id)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(pos.is_some(), "job should be in wait");

    assert_eq!(audit_entries_for(&db, conn_id, "promoteJob").await, 1);

    cleanup_queue(&mut conn, queue).await;
}

// ─── InvalidJobState — retry on non-failed ───────────────────────────────

#[tokio::test]
async fn retry_on_waiting_returns_invalid_job_state_and_no_audit() {
    if skip_if_no_local_redis() {
        return;
    }
    let (_d, db) = fresh_pool().await;
    let queue = "ranch-test-invalid-1";
    let job_id = "invalid-job-1";
    let conn_id = "conn-invalid-1";

    let mut conn = open_redis().await;
    cleanup_queue(&mut conn, queue).await;
    seed_job(&mut conn, queue, job_id, "waiting").await;

    let registry = ConnectionRegistry::new();
    let cfg = cfg_with(&db, conn_id, ReadOnlyState::Unlocked).await;
    let err = registry
        .retry_job(
            conn_id,
            &cfg,
            args_for(conn_id, queue, job_id),
            ranch_lib::core::connection_actor::AuditAttribution::ui(),
        )
        .await
        .expect("actor")
        .expect_err("expected InvalidJobState");

    match err {
        RanchError::InvalidJobState {
            action,
            current_state,
            required_state,
            ..
        } => {
            assert_eq!(action, "retry");
            assert_eq!(current_state, "waiting");
            assert_eq!(required_state, "failed");
        }
        other => panic!("expected InvalidJobState, got {other:?}"),
    }

    // No audit entry should have been written.
    assert_eq!(audit_entries_for(&db, conn_id, "retryJob").await, 0);

    cleanup_queue(&mut conn, queue).await;
}

// ─── JobNotFound — remove a fake id ──────────────────────────────────────

#[tokio::test]
async fn remove_nonexistent_returns_job_not_found_and_no_audit() {
    if skip_if_no_local_redis() {
        return;
    }
    let (_d, db) = fresh_pool().await;
    let queue = "ranch-test-notfound-1";
    let conn_id = "conn-notfound-1";

    let mut conn = open_redis().await;
    cleanup_queue(&mut conn, queue).await;
    // Seed only the meta so the schema is recognized; no job.
    redis::cmd("HSET")
        .arg(format!("bull:{queue}:meta"))
        .arg("version")
        .arg("bullmq:5.0.0")
        .exec_async(&mut conn)
        .await
        .unwrap();

    let registry = ConnectionRegistry::new();
    let cfg = cfg_with(&db, conn_id, ReadOnlyState::Unlocked).await;
    let err = registry
        .remove_job(
            conn_id,
            &cfg,
            args_for(conn_id, queue, "ghost"),
            ranch_lib::core::connection_actor::AuditAttribution::ui(),
        )
        .await
        .expect("actor")
        .expect_err("expected JobNotFound");
    assert!(matches!(err, RanchError::JobNotFound { .. }), "got {err:?}");

    assert_eq!(audit_entries_for(&db, conn_id, "removeJob").await, 0);

    cleanup_queue(&mut conn, queue).await;
}

// ─── ConnectionLocked — retry on a locked connection ─────────────────────

#[tokio::test]
async fn retry_on_locked_returns_connection_locked_and_no_audit() {
    if skip_if_no_local_redis() {
        return;
    }
    let (_d, db) = fresh_pool().await;
    let queue = "ranch-test-locked-1";
    let job_id = "locked-job-1";
    let conn_id = "conn-locked-1";

    let mut conn = open_redis().await;
    cleanup_queue(&mut conn, queue).await;
    seed_job(&mut conn, queue, job_id, "failed").await;

    let registry = ConnectionRegistry::new();
    let cfg = cfg_with(&db, conn_id, ReadOnlyState::Locked).await;
    let err = registry
        .retry_job(
            conn_id,
            &cfg,
            args_for(conn_id, queue, job_id),
            ranch_lib::core::connection_actor::AuditAttribution::ui(),
        )
        .await
        .expect("actor")
        .expect_err("expected ConnectionLocked");
    assert!(
        matches!(err, RanchError::ConnectionLocked { .. }),
        "got {err:?}"
    );

    // Lock-rejected single-action attempts do NOT write audit entries
    // (signal-to-noise; the bulk paths write one consolidated row). Confirm:
    assert_eq!(audit_entries_for(&db, conn_id, "retryJob").await, 0);

    // And critically: the job is still in `failed`, NOT moved to wait.
    let z: Option<f64> = redis::cmd("ZSCORE")
        .arg(format!("bull:{queue}:failed"))
        .arg(job_id)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(
        z.is_some(),
        "locked connection MUST NOT touch Redis — job should still be in failed"
    );

    cleanup_queue(&mut conn, queue).await;
}
