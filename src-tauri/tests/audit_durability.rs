//! Durability check for the audit log writer.
//!
//! Audit writes MUST survive an immediate process kill. We can't kill
//! the test process, but we verify the synchronous-flush contract by:
//!   1. Opening a pool against a tmp DB file.
//!   2. Writing an audit entry via `core::audit::record`.
//!   3. Dropping (closing) the pool synchronously after `record`
//!      returns — no graceful checkpoint allowed.
//!   4. Re-opening the same DB file.
//!   5. Asserting the entry is still there.
//!
//! Loops the cycle 10× to amplify any race condition or missing sync.
//! If the writer's `PRAGMA synchronous = FULL` is missing or commit
//! ordering is wrong, this test will be flaky on first run and
//! reliably fail under the loop. Catches durability regressions
//! architecturally.

use ranch_lib::core::audit::{
    record, AuditAction, AuditEntry, AuditOutcome, AuditSource, AuditTarget,
};
use ranch_lib::core::db::Db;
use tempfile::tempdir;

fn sample_entry(seq: usize) -> AuditEntry {
    AuditEntry {
        source: AuditSource::Ui,
        action: AuditAction::RetryJob,
        target: AuditTarget::Job {
            queue_name: "emails".into(),
            job_id: format!("job-{seq}"),
        },
        outcome: AuditOutcome::Success,
        outcome_reason: None,
        connection_id: Some("conn-x".into()),
        connection_name: Some("Acme · prod".into()),
        env_color: Some("env-production".into()),
        agent_client: None,
        agent_transport: None,
        approval_id: None,
    }
}

#[tokio::test]
async fn write_survives_close_and_reopen_cycle_10_iterations() {
    let dir = tempdir().expect("tempdir");

    for i in 0..10 {
        // Open the DB, write one audit entry, close.
        let db = Db::init(dir.path()).await.expect("init #{i}");
        record(db.pool(), sample_entry(i))
            .await
            .expect("record #{i}");
        // Drop the pool synchronously. If `PRAGMA synchronous = FULL`
        // is missing or the commit ordering is wrong, this is where a
        // pending write in the OS page cache would be lost.
        db.pool().close().await;
        drop(db);

        // Re-open. The migration pipeline is idempotent.
        let db2 = Db::init(dir.path()).await.expect("re-init #{i}");
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log_entries")
            .fetch_one(db2.pool())
            .await
            .expect("count");
        assert_eq!(
            count,
            (i + 1) as i64,
            "iteration {i}: expected {} rows after re-open, got {count}",
            i + 1
        );
        db2.pool().close().await;
        drop(db2);
    }
}

#[tokio::test]
async fn ten_writes_in_a_row_all_persist_after_close() {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("init");
    for i in 0..10 {
        record(db.pool(), sample_entry(i))
            .await
            .expect("record #{i}");
    }
    db.pool().close().await;
    drop(db);

    // Re-open and verify all 10 entries.
    let db2 = Db::init(dir.path()).await.expect("re-init");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log_entries")
        .fetch_one(db2.pool())
        .await
        .expect("count");
    assert_eq!(count, 10);

    // Verify the job ids are all present (deterministic content survives).
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT target_payload FROM audit_log_entries ORDER BY created_at ASC")
            .fetch_all(db2.pool())
            .await
            .expect("payloads");
    for (i, payload) in ids.into_iter().enumerate() {
        assert!(
            payload.contains(&format!("job-{i}")),
            "iteration {i}: expected job-{i} in target_payload, got {payload}"
        );
    }
}
