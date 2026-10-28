//! synchronous-flush audit writer.
//!
//! This module is private to `audit`. The [`record`] function is
//! re-exported from `audit` as the ONLY public way to write to the
//! `audit_log_entries` table.
//!
//! The write pipeline reaches it through its `AuditLog` port
//! (`SqliteAuditLog`), AFTER the backend acted and BEFORE the caller hears of
//! it; the lock toggles (the app crate's `commands::safety`) call it directly.
//! Module visibility prevents any other call site from bypassing the funnel.
//!
//! ## Durability contract
//!
//! The writer acquires a fresh pool connection, sets `PRAGMA
//! synchronous = FULL` BEFORE starting the transaction (SQLite forbids
//! changing the synchronous level inside a transaction), then opens a
//! tx, INSERTs the entry, and COMMITs. The function returns only after
//! commit. Result: a write that the user sees confirmed has been
//! fsync'd to disk before the confirmation appears.
//!
//! The app crate's `tests/audit_durability.rs` integration test loops
//! 10× through write → drop pool → reopen → assert-row-present to amplify
//! any race condition or missing sync.

use sqlx::{Acquire, Executor, SqlitePool};
use uuid::Uuid;

use ranch_domain::audit::{AuditEntry, AuditSource};
use ranch_domain::errors::RanchError;

/// Append one audit entry. Synchronous-flush durability.
///
/// Generates `id` (UUIDv7) and `created_at` (ISO-8601 UTC) inside the
/// call. `operator_id` is `"local"` for `AuditSource::Ui`; for
/// `AuditSource::Agent` it falls back to `agent_client` (or
/// `"agent"` if that's missing too).
///
/// Returns once the transaction has committed, i.e. the write is on
/// disk, not just in the OS page cache.
pub async fn record(pool: &SqlitePool, entry: AuditEntry) -> Result<(), RanchError> {
    // Acquire a dedicated connection so the PRAGMA + transaction
    // happen on the same connection. SQLite forbids changing the
    // synchronous level inside a transaction, so set the PRAGMA BEFORE
    // beginning the tx.
    let mut conn = pool.acquire().await.map_err(RanchError::database)?;
    conn.execute("PRAGMA synchronous = FULL")
        .await
        .map_err(RanchError::database)?;
    let mut tx = conn.begin().await.map_err(RanchError::database)?;

    let id = Uuid::now_v7().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    let operator_id = match entry.source {
        AuditSource::Ui => "local".to_string(),
        AuditSource::Agent => entry
            .agent_client
            .clone()
            .unwrap_or_else(|| "agent".to_string()),
    };
    let target_kind = entry.target.kind_str();
    let target_payload = serde_json::to_string(&entry.target)
        .expect("AuditTarget is always Serialize-safe (pure data)");

    sqlx::query(
        r#"
        INSERT INTO audit_log_entries (
            id, created_at, source, operator_id, agent_client, agent_transport,
            action, target_kind, target_payload,
            outcome, outcome_reason,
            connection_id, connection_name, env_color, approval_id
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(&created_at)
    .bind(entry.source.as_str())
    .bind(&operator_id)
    .bind(&entry.agent_client)
    .bind(&entry.agent_transport)
    .bind(entry.action.as_str())
    .bind(target_kind)
    .bind(&target_payload)
    .bind(entry.outcome.as_str())
    .bind(&entry.outcome_reason)
    .bind(&entry.connection_id)
    .bind(&entry.connection_name)
    .bind(&entry.env_color)
    .bind(&entry.approval_id)
    .execute(&mut *tx)
    .await
    .map_err(RanchError::database)?;

    tx.commit().await.map_err(RanchError::database)?;
    drop(conn);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ranch_domain::audit::{AuditAction, AuditOutcome, AuditTarget};
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        // In-memory DB for unit tests: no fsync to disk needed, the
        // PRAGMA still validates the writer code path.
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            // In-memory SQLite is per-connection: pin to 1 so all queries
            // see the same DB.
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn sample_entry() -> AuditEntry {
        AuditEntry {
            source: AuditSource::Ui,
            action: AuditAction::RetryJob,
            target: AuditTarget::job(
                &ranch_domain::queues::QueueId::new("bull:emails"),
                "emails",
                "42",
            ),
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
    async fn happy_path_inserts_a_row_visible_via_select() {
        let pool = fresh_pool().await;
        record(&pool, sample_entry()).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log_entries")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn ui_source_maps_to_local_operator_id() {
        let pool = fresh_pool().await;
        record(&pool, sample_entry()).await.unwrap();
        let operator: String =
            sqlx::query_scalar("SELECT operator_id FROM audit_log_entries LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(operator, "local");
    }

    #[tokio::test]
    async fn agent_source_maps_agent_client_to_operator_id() {
        let pool = fresh_pool().await;
        let mut e = sample_entry();
        e.source = AuditSource::Agent;
        e.agent_client = Some("claude-desktop".into());
        e.agent_transport = Some("stdio".into());
        record(&pool, e).await.unwrap();
        let row: (String, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT operator_id, agent_client, agent_transport FROM audit_log_entries LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, "claude-desktop");
        assert_eq!(row.1.as_deref(), Some("claude-desktop"));
        assert_eq!(row.2.as_deref(), Some("stdio"));
    }

    #[tokio::test]
    async fn target_payload_round_trips_through_json_column() {
        let pool = fresh_pool().await;
        record(&pool, sample_entry()).await.unwrap();
        let payload: String =
            sqlx::query_scalar("SELECT target_payload FROM audit_log_entries LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: AuditTarget = serde_json::from_str(&payload).unwrap();
        assert_eq!(
            parsed,
            AuditTarget::job(
                &ranch_domain::queues::QueueId::new("bull:emails"),
                "emails",
                "42",
            )
        );
    }

    #[tokio::test]
    async fn writes_use_camelcase_action_discriminator() {
        let pool = fresh_pool().await;
        let mut e = sample_entry();
        e.action = AuditAction::PauseScheduler;
        record(&pool, e).await.unwrap();
        let action: String = sqlx::query_scalar("SELECT action FROM audit_log_entries LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(action, "pauseScheduler");
    }
}
