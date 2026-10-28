//! Live integration tests for bulk single-queue actions
//! (bulk retry / bulk remove / bulk promote).
//!
//! Ignored unless run with `--include-ignored` and `RANCH_LOCAL_REDIS=1`. Each test seeds its own
//! isolated queue.
//!
//! ```bash
//! docker compose --profile redis up -d
//! RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 \
//!   cargo test --test bulk_actions_live -- --include-ignored --nocapture
//! ```
//!
//! What each test asserts:
//!   - The bulk action's Redis effect across N jobs.
//!   - ONE consolidated audit row written via `adapters::sqlite::audit::record`
//!     with `target_kind = "bulkJobs"`.
//!   - Partial-failure collection: state-mismatch jobs end up in
//!     `failed`, others succeed, the loop doesn't abort.
//!   - Lock-rejected bulk: ONE consolidated audit row with
//!     `outcome=failure, reason="Connection locked"`. NO Redis touched.
//!
//! Progress events are NOT asserted here: the live tests bypass
//! Tauri (no AppHandle) so the actor's `if let Some(a) = app` arm
//! is skipped. Frontend tests cover the event-listener wiring.

use ranch_lib::adapters::bullmq_redis::bullmq_schema::QueueRef;
use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::jobs::{
    bulk_promote_jobs_inner, bulk_remove_jobs_inner, bulk_retry_jobs_inner, BulkActionArgs,
};
use ranch_lib::domain::errors::RanchError;
use tempfile::tempdir;

mod common;
use common::bullmq_seed::{clear_queue, seed_job};
use common::no_progress;
use common::repo;
use common::{audit, redis_target};

async fn fresh_setup() -> (
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

    // Persist a connection so the inner-fn resolve path works.
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
            environment: EnvironmentSelector::for_color("env-local"),
        },
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    (dir, db, kc, registry, conn.id)
}

/// Variant of fresh_setup that creates a production-color connection
/// (production connections are locked by default).
async fn fresh_setup_locked() -> (
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
            name: "test-prod".into(),
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
            environment: EnvironmentSelector::for_color("env-production"),
        },
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    (dir, db, kc, registry, conn.id)
}

// ─── Happy paths ─────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn bulk_retry_succeeds_for_all_failed_jobs() {
    let (_d, db, kc, reg, conn_id) = fresh_setup().await;
    let queue = "ranch-test-bulk-retry-1";

    let mut rconn = redis_target::connect().await;
    clear_queue(&mut rconn, queue).await;
    let job_ids: Vec<String> = (1..=10).map(|i| format!("retry-{i}")).collect();
    for jid in &job_ids {
        seed_job(&mut rconn, queue, jid, "failed").await;
    }

    let result = bulk_retry_jobs_inner(
        BulkActionArgs {
            connection_id: conn_id.clone(),
            queue_id: QueueRef {
                prefix: "bull".into(),
                name: queue.into(),
            }
            .id()
            .into(),
            job_ids: job_ids.clone(),
        },
        &common::backends(db.pool(), &kc, &reg),
        no_progress(),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .unwrap();

    assert_eq!(result.total, 10);
    assert_eq!(result.succeeded, 10);
    assert!(result.failed.is_empty());
    assert!(!result.cancelled);

    // All jobs should now be in `wait`, not in `failed`.
    let failed_count: u64 = redis::cmd("ZCARD")
        .arg(format!("bull:{queue}:failed"))
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(failed_count, 0);
    let wait_count: u64 = redis::cmd("LLEN")
        .arg(format!("bull:{queue}:wait"))
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(wait_count, 10);

    let rows = audit::rows(db.pool(), &conn_id).await;
    assert_eq!(rows.len(), 1, "exactly ONE consolidated audit row");
    let row = &rows[0];
    assert_eq!(row.action, ranch_lib::domain::audit::AuditAction::RetryJob);
    assert_eq!(
        row.target,
        ranch_lib::domain::audit::AuditTarget::bulk_jobs(
            &ranch_lib::domain::queues::QueueId::new(format!("bull:{queue}")),
            queue,
            10,
        )
    );
    assert_eq!(row.outcome, ranch_lib::domain::audit::AuditOutcome::Success);

    clear_queue(&mut rconn, queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn bulk_retry_partial_failure_collected() {
    let (_d, db, kc, reg, conn_id) = fresh_setup().await;
    let queue = "ranch-test-bulk-partial-1";

    let mut rconn = redis_target::connect().await;
    clear_queue(&mut rconn, queue).await;
    // 8 failed jobs + 2 in waiting (precondition mismatch).
    for i in 1..=8 {
        seed_job(&mut rconn, queue, &format!("ok-{i}"), "failed").await;
    }
    for i in 1..=2 {
        seed_job(&mut rconn, queue, &format!("bad-{i}"), "waiting").await;
    }
    let mut job_ids: Vec<String> = (1..=8).map(|i| format!("ok-{i}")).collect();
    job_ids.extend((1..=2).map(|i| format!("bad-{i}")));

    let result = bulk_retry_jobs_inner(
        BulkActionArgs {
            connection_id: conn_id.clone(),
            queue_id: QueueRef {
                prefix: "bull".into(),
                name: queue.into(),
            }
            .id()
            .into(),
            job_ids,
        },
        &common::backends(db.pool(), &kc, &reg),
        no_progress(),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .unwrap();

    assert_eq!(result.total, 10);
    assert_eq!(result.succeeded, 8);
    assert_eq!(result.failed.len(), 2);
    let failed_ids: Vec<&str> = result.failed.iter().map(|f| f.job_id.as_str()).collect();
    assert!(failed_ids.contains(&"bad-1"));
    assert!(failed_ids.contains(&"bad-2"));

    let rows = audit::rows(db.pool(), &conn_id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].outcome,
        ranch_lib::domain::audit::AuditOutcome::Failure
    );

    clear_queue(&mut rconn, queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn bulk_remove_deletes_all_hashes() {
    let (_d, db, kc, reg, conn_id) = fresh_setup().await;
    let queue = "ranch-test-bulk-remove-1";

    let mut rconn = redis_target::connect().await;
    clear_queue(&mut rconn, queue).await;
    let job_ids: Vec<String> = (1..=10).map(|i| format!("rm-{i}")).collect();
    for jid in &job_ids {
        seed_job(&mut rconn, queue, jid, "failed").await;
    }

    let result = bulk_remove_jobs_inner(
        BulkActionArgs {
            connection_id: conn_id.clone(),
            queue_id: QueueRef {
                prefix: "bull".into(),
                name: queue.into(),
            }
            .id()
            .into(),
            job_ids: job_ids.clone(),
        },
        &common::backends(db.pool(), &kc, &reg),
        no_progress(),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .unwrap();

    assert_eq!(result.succeeded, 10);
    assert!(result.failed.is_empty());

    for jid in &job_ids {
        let exists: i64 = redis::cmd("EXISTS")
            .arg(format!("bull:{queue}:{jid}"))
            .query_async(&mut rconn)
            .await
            .unwrap();
        assert_eq!(exists, 0, "job {jid} hash should be gone");
    }

    let rows = audit::rows(db.pool(), &conn_id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].action,
        ranch_lib::domain::audit::AuditAction::RemoveJob
    );

    clear_queue(&mut rconn, queue).await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn bulk_promote_moves_all_delayed_to_waiting() {
    let (_d, db, kc, reg, conn_id) = fresh_setup().await;
    let queue = "ranch-test-bulk-promote-1";

    let mut rconn = redis_target::connect().await;
    clear_queue(&mut rconn, queue).await;
    let job_ids: Vec<String> = (1..=5).map(|i| format!("p-{i}")).collect();
    for jid in &job_ids {
        seed_job(&mut rconn, queue, jid, "delayed").await;
    }

    let result = bulk_promote_jobs_inner(
        BulkActionArgs {
            connection_id: conn_id.clone(),
            queue_id: QueueRef {
                prefix: "bull".into(),
                name: queue.into(),
            }
            .id()
            .into(),
            job_ids,
        },
        &common::backends(db.pool(), &kc, &reg),
        no_progress(),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .unwrap();

    assert_eq!(result.succeeded, 5);
    let delayed_count: u64 = redis::cmd("ZCARD")
        .arg(format!("bull:{queue}:delayed"))
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(delayed_count, 0);
    let wait_count: u64 = redis::cmd("LLEN")
        .arg(format!("bull:{queue}:wait"))
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(wait_count, 5);

    clear_queue(&mut rconn, queue).await;
}

// ─── Lock rejection: the canary ──────────────────────────────────────

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn bulk_locked_connection_rejects_with_consolidated_audit() {
    let (_d, db, kc, reg, conn_id) = fresh_setup_locked().await;
    let queue = "ranch-test-bulk-locked-1";

    let mut rconn = redis_target::connect().await;
    clear_queue(&mut rconn, queue).await;
    let job_ids: Vec<String> = (1..=10).map(|i| format!("locked-{i}")).collect();
    for jid in &job_ids {
        seed_job(&mut rconn, queue, jid, "failed").await;
    }

    let err = bulk_retry_jobs_inner(
        BulkActionArgs {
            connection_id: conn_id.clone(),
            queue_id: QueueRef {
                prefix: "bull".into(),
                name: queue.into(),
            }
            .id()
            .into(),
            job_ids: job_ids.clone(),
        },
        &common::backends(db.pool(), &kc, &reg),
        no_progress(),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, RanchError::ConnectionLocked { .. }),
        "got {err:?}"
    );

    // Consolidated audit row written by the command-level wrapper.
    let rows = audit::rows(db.pool(), &conn_id).await;
    assert_eq!(rows.len(), 1, "exactly ONE consolidated lock-rejection row");
    let row = &rows[0];
    assert_eq!(row.action, ranch_lib::domain::audit::AuditAction::RetryJob);
    assert_eq!(
        row.target,
        ranch_lib::domain::audit::AuditTarget::bulk_jobs(
            &ranch_lib::domain::queues::QueueId::new(format!("bull:{queue}")),
            queue,
            10,
        )
    );
    assert_eq!(row.outcome, ranch_lib::domain::audit::AuditOutcome::Failure);
    assert_eq!(row.outcome_reason.as_deref(), Some("Connection locked"),);

    // AND, critically, no Redis mutation happened. All 10 still in `failed`.
    let failed_count: u64 = redis::cmd("ZCARD")
        .arg(format!("bull:{queue}:failed"))
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(failed_count, 10, "locked connection MUST NOT touch Redis");

    clear_queue(&mut rconn, queue).await;
}

// ─── Smoke for chunking (verifies BULK_PROGRESS_CHUNK doesn't break) ─────

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn bulk_retry_with_30_jobs_completes_without_panic() {
    let (_d, db, kc, reg, conn_id) = fresh_setup().await;
    let queue = "ranch-test-bulk-chunk-1";

    let mut rconn = redis_target::connect().await;
    clear_queue(&mut rconn, queue).await;
    let job_ids: Vec<String> = (1..=30).map(|i| format!("chunk-{i}")).collect();
    for jid in &job_ids {
        seed_job(&mut rconn, queue, jid, "failed").await;
    }

    // app: None: progress events are skipped but the loop still runs
    // through the chunk boundary at 10, 20, 30. The handler must
    // complete cleanly. If a future refactor breaks the chunk math,
    // this test catches it.
    let result = bulk_retry_jobs_inner(
        BulkActionArgs {
            connection_id: conn_id.clone(),
            queue_id: QueueRef {
                prefix: "bull".into(),
                name: queue.into(),
            }
            .id()
            .into(),
            job_ids,
        },
        &common::backends(db.pool(), &kc, &reg),
        no_progress(),
        ranch_lib::domain::audit::Caller::User,
    )
    .await
    .unwrap();
    assert_eq!(result.succeeded, 30);

    clear_queue(&mut rconn, queue).await;
}
