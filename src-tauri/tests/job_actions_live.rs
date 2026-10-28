//! Live integration tests for single-job actions
//! (retry / remove / promote).
//!
//! Ignored unless run with `--include-ignored` and `RANCH_LOCAL_REDIS=1`. Each test seeds its own
//! isolated job into a unique queue (`ranch-test-action-{action}-{n}`)
//! to avoid cross-test interference with the BullMQ seed used by the
//! other live tests.
//!
//! ```bash
//! docker compose --profile redis up -d
//! RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 \
//!   cargo test --test job_actions_live -- --include-ignored --nocapture
//! ```
//!
//! What each test asserts:
//!   - The action's Redis effect (state transition / hash deletion).
//!   - An audit entry is written via `adapters::sqlite::audit::record`.
//!   - State-precondition violations return `InvalidJobState` and do
//!     NOT write audit entries.
//!   - Lock rejection returns `ConnectionLocked` BEFORE any Redis op.

use ranch_lib::adapters::bullmq_redis::bullmq_schema::{JobState, QueueRef};
use ranch_lib::adapters::bullmq_redis::connection_actor::{ConnectionConfig, ConnectionRegistry};
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::domain::audit::{AuditOutcome, AuditSource};
use ranch_lib::domain::errors::RanchError;
use ranch_lib::domain::jobs::JobActionResult;
use ranch_lib::domain::jobs::JobTarget;
use ranch_lib::domain::lock::ReadOnlyState;
use tempfile::tempdir;

mod common;
use common::bullmq_seed::{clear_queue, seed_job};
use common::{audit, redis_target};

async fn fresh_pool() -> (tempfile::TempDir, Db) {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    (dir, db)
}

async fn cfg_with(state_db: &Db, conn_id: &str, lock_state: ReadOnlyState) -> ConnectionConfig {
    ConnectionConfig {
        endpoint: ranch_lib::adapters::bullmq_redis::endpoint::RedisEndpoint {
            host: redis_target::host(),
            port: redis_target::port(),
            db: 0,
            username: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
        },
        password: None,
        ssh_password: None,
        keychain: std::sync::Arc::new(ranch_lib::adapters::secrets::InMemoryKeychain::new()),
        initial_lock_state: lock_state,
        connection_id: conn_id.into(),
        connection_name: format!("Test · {conn_id}"),
        env_color: Some("env-local".into()),
        writes: ranch_lib::adapters::sqlite::write_pipeline(&state_db.pool().clone()),
    }
}

fn args_for(queue: &str, job_id: &str) -> JobTarget {
    JobTarget {
        queue_id: QueueRef {
            prefix: "bull".into(),
            name: queue.into(),
        }
        .id(),
        job_id: job_id.into(),
    }
}

// ─── Retry: happy path ───────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn retry_moves_failed_job_to_waiting_and_writes_audit() {
    let (_d, db) = fresh_pool().await;
    let queue = "ranch-test-retry-1";
    let job_id = "retry-job-1";
    let conn_id = "conn-retry-1";

    let mut conn = redis_target::connect().await;
    clear_queue(&mut conn, queue).await;
    seed_job(&mut conn, queue, job_id, "failed").await;

    let registry = ConnectionRegistry::new();
    let cfg = cfg_with(&db, conn_id, ReadOnlyState::Unlocked).await;
    let result: JobActionResult = registry
        .retry_job(
            conn_id,
            &cfg,
            args_for(queue, job_id),
            ranch_lib::domain::audit::Caller::User,
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

    assert_eq!(audit::count(db.pool(), conn_id, "retryJob").await, 1);
    // A desktop write is signed by the user, with no agent fields.
    let row = &audit::rows(db.pool(), conn_id).await[0];
    assert_eq!(row.source, AuditSource::Ui);
    assert_eq!(
        (row.agent_client.as_deref(), row.agent_transport.as_deref()),
        (None, None)
    );
    assert_eq!(row.outcome, AuditOutcome::Success);
    assert_eq!(row.approval_id, None);

    clear_queue(&mut conn, queue).await;
}

// ─── Remove: happy path ──────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn remove_deletes_hash_and_writes_audit() {
    let (_d, db) = fresh_pool().await;
    let queue = "ranch-test-remove-1";
    let job_id = "remove-job-1";
    let conn_id = "conn-remove-1";

    let mut conn = redis_target::connect().await;
    clear_queue(&mut conn, queue).await;
    seed_job(&mut conn, queue, job_id, "failed").await;

    let registry = ConnectionRegistry::new();
    let cfg = cfg_with(&db, conn_id, ReadOnlyState::Unlocked).await;
    let result = registry
        .remove_job(
            conn_id,
            &cfg,
            args_for(queue, job_id),
            ranch_lib::domain::audit::Caller::User,
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

    assert_eq!(audit::count(db.pool(), conn_id, "removeJob").await, 1);

    clear_queue(&mut conn, queue).await;
}

// ─── Promote: happy path ─────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn promote_moves_delayed_job_to_waiting_and_writes_audit() {
    let (_d, db) = fresh_pool().await;
    let queue = "ranch-test-promote-1";
    let job_id = "promote-job-1";
    let conn_id = "conn-promote-1";

    let mut conn = redis_target::connect().await;
    clear_queue(&mut conn, queue).await;
    seed_job(&mut conn, queue, job_id, "delayed").await;

    let registry = ConnectionRegistry::new();
    let cfg = cfg_with(&db, conn_id, ReadOnlyState::Unlocked).await;
    let result = registry
        .promote_job(
            conn_id,
            &cfg,
            args_for(queue, job_id),
            ranch_lib::domain::audit::Caller::User,
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

    assert_eq!(audit::count(db.pool(), conn_id, "promoteJob").await, 1);

    clear_queue(&mut conn, queue).await;
}

// ─── InvalidJobState: retry on non-failed ────────────────────────────────

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn retry_on_waiting_returns_invalid_job_state_and_no_audit() {
    let (_d, db) = fresh_pool().await;
    let queue = "ranch-test-invalid-1";
    let job_id = "invalid-job-1";
    let conn_id = "conn-invalid-1";

    let mut conn = redis_target::connect().await;
    clear_queue(&mut conn, queue).await;
    seed_job(&mut conn, queue, job_id, "waiting").await;

    let registry = ConnectionRegistry::new();
    let cfg = cfg_with(&db, conn_id, ReadOnlyState::Unlocked).await;
    let err = registry
        .retry_job(
            conn_id,
            &cfg,
            args_for(queue, job_id),
            ranch_lib::domain::audit::Caller::User,
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
            assert_eq!(current_state, JobState::Waiting);
            assert_eq!(required_state, JobState::Failed);
        }
        other => panic!("expected InvalidJobState, got {other:?}"),
    }

    // No audit entry should have been written.
    assert_eq!(audit::count(db.pool(), conn_id, "retryJob").await, 0);

    clear_queue(&mut conn, queue).await;
}

// ─── JobNotFound: remove a fake id ───────────────────────────────────────

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn remove_nonexistent_returns_job_not_found_and_no_audit() {
    let (_d, db) = fresh_pool().await;
    let queue = "ranch-test-notfound-1";
    let conn_id = "conn-notfound-1";

    let mut conn = redis_target::connect().await;
    clear_queue(&mut conn, queue).await;
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
            args_for(queue, "ghost"),
            ranch_lib::domain::audit::Caller::User,
        )
        .await
        .expect("actor")
        .expect_err("expected JobNotFound");
    assert!(matches!(err, RanchError::JobNotFound { .. }), "got {err:?}");

    assert_eq!(audit::count(db.pool(), conn_id, "removeJob").await, 0);

    clear_queue(&mut conn, queue).await;
}

// ─── ConnectionLocked: retry on a locked connection ──────────────────────

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn retry_on_locked_returns_connection_locked_and_no_audit() {
    let (_d, db) = fresh_pool().await;
    let queue = "ranch-test-locked-1";
    let job_id = "locked-job-1";
    let conn_id = "conn-locked-1";

    let mut conn = redis_target::connect().await;
    clear_queue(&mut conn, queue).await;
    seed_job(&mut conn, queue, job_id, "failed").await;

    let registry = ConnectionRegistry::new();
    let cfg = cfg_with(&db, conn_id, ReadOnlyState::Locked).await;
    let err = registry
        .retry_job(
            conn_id,
            &cfg,
            args_for(queue, job_id),
            ranch_lib::domain::audit::Caller::User,
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
    assert_eq!(audit::count(db.pool(), conn_id, "retryJob").await, 0);

    // And critically: the job is still in `failed`, NOT moved to wait.
    let z: Option<f64> = redis::cmd("ZSCORE")
        .arg(format!("bull:{queue}:failed"))
        .arg(job_id)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(
        z.is_some(),
        "locked connection MUST NOT touch Redis, job should still be in failed"
    );

    clear_queue(&mut conn, queue).await;
}
