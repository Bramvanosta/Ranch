//! Reading back the audit log a test produced.

use ranch_lib::commands::audit::{read_audit_log, ListAuditLogEntriesArgs};
use ranch_lib::domain::audit::AuditLogEntry;
use sqlx::SqlitePool;

/// Every row for `connection_id`, newest first.
pub async fn rows(pool: &SqlitePool, connection_id: &str) -> Vec<AuditLogEntry> {
    read_audit_log(
        ListAuditLogEntriesArgs {
            from: None,
            to: None,
            connection_id: Some(connection_id.into()),
            action: None,
            source: None,
            offset: 0,
            limit: 1000,
        },
        pool,
    )
    .await
    .unwrap()
}

/// How many rows `connection_id` has for `action` (`"retryJob"`…).
pub async fn count(pool: &SqlitePool, connection_id: &str, action: &str) -> usize {
    rows(pool, connection_id)
        .await
        .iter()
        .filter(|r| r.action.as_str() == action)
        .count()
}
