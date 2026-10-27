//! Live integration tests for bulk single-queue actions
//! (bulk retry / bulk remove / bulk promote).
//!
//! Skips unless `RANCH_LOCAL_REDIS=1` is set. Each test seeds its own
//! isolated queue.
//!
//! ```bash
//! docker compose --profile redis up -d
//! RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 \
//!   cargo test --test bulk_actions_live -- --nocapture
//! ```
//!
//! What each test asserts:
//!   - The bulk action's Redis effect across N jobs.
//!   - ONE consolidated audit row written via `core::audit::record`
//!     with `target_kind = "bulkJobs"`.
//!   - Partial-failure collection — state-mismatch jobs end up in
//!     `failed`, others succeed, the loop doesn't abort.
//!   - Lock-rejected bulk: ONE consolidated audit row with
//!     `outcome=failure, reason="Connection locked"`. NO Redis touched.
//!
//! Progress events are NOT asserted here — the live tests bypass
//! Tauri (no AppHandle) so the actor's `if let Some(a) = app` arm
//! is skipped. Frontend tests cover the event-listener wiring.

use ranch_lib::commands::audit::{list_audit_log_entries_inner, ListAuditLogEntriesArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::jobs::{
    bulk_promote_jobs_inner, bulk_remove_jobs_inner, bulk_retry_jobs_inner, BulkActionArgs,
};
use ranch_lib::core::bullmq_schema::QueueRef;
use ranch_lib::core::connection_actor::ConnectionRegistry;
use ranch_lib::core::db::Db;
use ranch_lib::core::errors::RanchError;
use ranch_lib::core::keychain::InMemoryKeychain;
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

async fn fresh_setup() -> (
    tempfile::TempDir,
    Db,
    std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter>,
    ConnectionRegistry,
    String,
) {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());
    let registry = ConnectionRegistry::new();

    // Persist a connection so the inner-fn resolve path works.
    let conn = ranch_lib::commands::connections::add_connection_inner(
        ranch_lib::commands::connections::AddConnectionArgs {
            locked: None,
            name: "test".into(),
            host: redis_host(),
            port: redis_port(),
            db: 0,
            username: None,
            password: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
            ssh_password: None,
            group: GroupSelector::named("Default"),
            environment: EnvironmentSelector::for_color("env-local"),
        },
        db.pool(),
        &kc,
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
    std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter>,
    ConnectionRegistry,
    String,
) {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());
    let registry = ConnectionRegistry::new();
    let conn = ranch_lib::commands::connections::add_connection_inner(
        ranch_lib::commands::connections::AddConnectionArgs {
            locked: None,
            name: "test-prod".into(),
            host: redis_host(),
            port: redis_port(),
            db: 0,
            username: None,
            password: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
            ssh_password: None,
            group: GroupSelector::named("Default"),
            environment: EnvironmentSelector::for_color("env-production"),
        },
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    (dir, db, kc, registry, conn.id)
}

async fn open_redis() -> redis::aio::MultiplexedConnection {
    let url = format!("redis://{}:{}", redis_host(), redis_port());
    let client = redis::Client::open(url).expect("redis::Client::open");
    client
        .get_multiplexed_async_connection()
        .await
        .expect("multiplexed conn")
}

async fn seed_job(
    conn: &mut redis::aio::MultiplexedConnection,
    queue: &str,
    job_id: &str,
    state: &str,
) {
    let meta_key = format!("bull:{queue}:meta");
    let job_hash_key = format!("bull:{queue}:{job_id}");

    redis::cmd("HSET")
        .arg(&meta_key)
        .arg("version")
        .arg("bullmq:5.0.0")
        .exec_async(conn)
        .await
        .unwrap();

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

    let container_key = match state {
        "waiting" => format!("bull:{queue}:wait"),
        "failed" => format!("bull:{queue}:failed"),
        "delayed" => format!("bull:{queue}:delayed"),
        other => panic!("unknown state: {other}"),
    };
    match state {
        "waiting" => {
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
    let mut scan = redis::cmd("KEYS");
    scan.arg(format!("bull:{queue}:*"));
    let keys: Vec<String> = scan.query_async(conn).await.unwrap_or_default();
    for k in keys {
        let _ = redis::cmd("DEL").arg(&k).exec_async(conn).await;
    }
}

async fn audit_rows(db: &Db, conn_id: &str) -> Vec<ranch_lib::core::audit::AuditLogEntry> {
    list_audit_log_entries_inner(
        ListAuditLogEntriesArgs {
            from: None,
            to: None,
            connection_id: Some(conn_id.into()),
            action: None,
            source: None,
            offset: 0,
            limit: 100,
        },
        db.pool(),
    )
    .await
    .unwrap()
}

// ─── Happy paths ─────────────────────────────────────────────────────────

#[tokio::test]
async fn bulk_retry_succeeds_for_all_failed_jobs() {
    if skip_if_no_local_redis() {
        return;
    }
    let (_d, db, kc, reg, conn_id) = fresh_setup().await;
    let queue = "ranch-test-bulk-retry-1";

    let mut rconn = open_redis().await;
    cleanup_queue(&mut rconn, queue).await;
    let job_ids: Vec<String> = (1..=10).map(|i| format!("retry-{i}")).collect();
    for jid in &job_ids {
        seed_job(&mut rconn, queue, jid, "failed").await;
    }

    let result = bulk_retry_jobs_inner(
        BulkActionArgs {
            connection_id: conn_id.clone(),
            queue_ref: QueueRef {
                prefix: "bull".into(),
                name: queue.into(),
            },
            job_ids: job_ids.clone(),
        },
        db.pool(),
        &kc,
        &reg,
        None,
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

    let rows = audit_rows(&db, &conn_id).await;
    assert_eq!(rows.len(), 1, "exactly ONE consolidated audit row");
    let row = &rows[0];
    assert_eq!(row.action, ranch_lib::core::audit::AuditAction::RetryJob);
    assert_eq!(
        row.target,
        ranch_lib::core::audit::AuditTarget::BulkJobs {
            queue_name: queue.into(),
            job_count: 10,
        }
    );
    assert_eq!(row.outcome, ranch_lib::core::audit::AuditOutcome::Success);

    cleanup_queue(&mut rconn, queue).await;
}

#[tokio::test]
async fn bulk_retry_partial_failure_collected() {
    if skip_if_no_local_redis() {
        return;
    }
    let (_d, db, kc, reg, conn_id) = fresh_setup().await;
    let queue = "ranch-test-bulk-partial-1";

    let mut rconn = open_redis().await;
    cleanup_queue(&mut rconn, queue).await;
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
            queue_ref: QueueRef {
                prefix: "bull".into(),
                name: queue.into(),
            },
            job_ids,
        },
        db.pool(),
        &kc,
        &reg,
        None,
    )
    .await
    .unwrap();

    assert_eq!(result.total, 10);
    assert_eq!(result.succeeded, 8);
    assert_eq!(result.failed.len(), 2);
    let failed_ids: Vec<&str> = result.failed.iter().map(|f| f.job_id.as_str()).collect();
    assert!(failed_ids.contains(&"bad-1"));
    assert!(failed_ids.contains(&"bad-2"));

    let rows = audit_rows(&db, &conn_id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].outcome,
        ranch_lib::core::audit::AuditOutcome::Failure
    );

    cleanup_queue(&mut rconn, queue).await;
}

#[tokio::test]
async fn bulk_remove_deletes_all_hashes() {
    if skip_if_no_local_redis() {
        return;
    }
    let (_d, db, kc, reg, conn_id) = fresh_setup().await;
    let queue = "ranch-test-bulk-remove-1";

    let mut rconn = open_redis().await;
    cleanup_queue(&mut rconn, queue).await;
    let job_ids: Vec<String> = (1..=10).map(|i| format!("rm-{i}")).collect();
    for jid in &job_ids {
        seed_job(&mut rconn, queue, jid, "failed").await;
    }

    let result = bulk_remove_jobs_inner(
        BulkActionArgs {
            connection_id: conn_id.clone(),
            queue_ref: QueueRef {
                prefix: "bull".into(),
                name: queue.into(),
            },
            job_ids: job_ids.clone(),
        },
        db.pool(),
        &kc,
        &reg,
        None,
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

    let rows = audit_rows(&db, &conn_id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].action,
        ranch_lib::core::audit::AuditAction::RemoveJob
    );

    cleanup_queue(&mut rconn, queue).await;
}

#[tokio::test]
async fn bulk_promote_moves_all_delayed_to_waiting() {
    if skip_if_no_local_redis() {
        return;
    }
    let (_d, db, kc, reg, conn_id) = fresh_setup().await;
    let queue = "ranch-test-bulk-promote-1";

    let mut rconn = open_redis().await;
    cleanup_queue(&mut rconn, queue).await;
    let job_ids: Vec<String> = (1..=5).map(|i| format!("p-{i}")).collect();
    for jid in &job_ids {
        seed_job(&mut rconn, queue, jid, "delayed").await;
    }

    let result = bulk_promote_jobs_inner(
        BulkActionArgs {
            connection_id: conn_id.clone(),
            queue_ref: QueueRef {
                prefix: "bull".into(),
                name: queue.into(),
            },
            job_ids,
        },
        db.pool(),
        &kc,
        &reg,
        None,
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

    cleanup_queue(&mut rconn, queue).await;
}

// ─── Lock rejection — the canary ─────────────────────────────────────

#[tokio::test]
async fn bulk_locked_connection_rejects_with_consolidated_audit() {
    if skip_if_no_local_redis() {
        return;
    }
    let (_d, db, kc, reg, conn_id) = fresh_setup_locked().await;
    let queue = "ranch-test-bulk-locked-1";

    let mut rconn = open_redis().await;
    cleanup_queue(&mut rconn, queue).await;
    let job_ids: Vec<String> = (1..=10).map(|i| format!("locked-{i}")).collect();
    for jid in &job_ids {
        seed_job(&mut rconn, queue, jid, "failed").await;
    }

    let err = bulk_retry_jobs_inner(
        BulkActionArgs {
            connection_id: conn_id.clone(),
            queue_ref: QueueRef {
                prefix: "bull".into(),
                name: queue.into(),
            },
            job_ids: job_ids.clone(),
        },
        db.pool(),
        &kc,
        &reg,
        None,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, RanchError::ConnectionLocked { .. }),
        "got {err:?}"
    );

    // Consolidated audit row written by the command-level wrapper.
    let rows = audit_rows(&db, &conn_id).await;
    assert_eq!(rows.len(), 1, "exactly ONE consolidated lock-rejection row");
    let row = &rows[0];
    assert_eq!(row.action, ranch_lib::core::audit::AuditAction::RetryJob);
    assert_eq!(
        row.target,
        ranch_lib::core::audit::AuditTarget::BulkJobs {
            queue_name: queue.into(),
            job_count: 10,
        }
    );
    assert_eq!(row.outcome, ranch_lib::core::audit::AuditOutcome::Failure);
    assert_eq!(row.outcome_reason.as_deref(), Some("Connection locked"),);

    // AND — critically — no Redis mutation happened. All 10 still in `failed`.
    let failed_count: u64 = redis::cmd("ZCARD")
        .arg(format!("bull:{queue}:failed"))
        .query_async(&mut rconn)
        .await
        .unwrap();
    assert_eq!(failed_count, 10, "locked connection MUST NOT touch Redis");

    cleanup_queue(&mut rconn, queue).await;
}

// ─── Smoke for chunking (verifies BULK_PROGRESS_CHUNK doesn't break) ─────

#[tokio::test]
async fn bulk_retry_with_30_jobs_completes_without_panic() {
    if skip_if_no_local_redis() {
        return;
    }
    let (_d, db, kc, reg, conn_id) = fresh_setup().await;
    let queue = "ranch-test-bulk-chunk-1";

    let mut rconn = open_redis().await;
    cleanup_queue(&mut rconn, queue).await;
    let job_ids: Vec<String> = (1..=30).map(|i| format!("chunk-{i}")).collect();
    for jid in &job_ids {
        seed_job(&mut rconn, queue, jid, "failed").await;
    }

    // app: None — progress events are skipped but the loop still runs
    // through the chunk boundary at 10, 20, 30. The handler must
    // complete cleanly. If a future refactor breaks the chunk math,
    // this test catches it.
    let result = bulk_retry_jobs_inner(
        BulkActionArgs {
            connection_id: conn_id.clone(),
            queue_ref: QueueRef {
                prefix: "bull".into(),
                name: queue.into(),
            },
            job_ids,
        },
        db.pool(),
        &kc,
        &reg,
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.succeeded, 30);

    cleanup_queue(&mut rconn, queue).await;
}
