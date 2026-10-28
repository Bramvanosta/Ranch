//! The approval queue's last transition, `approved → consumed`: what the
//! write pipeline trades an approval with (`ApprovalStore`). The queue's
//! other transitions, and the policy around them, are in `core::mcp::approval`
//! of the app crate; the state machine is in
//! `migrations/0006_mcp_pending_approvals.sql`. Each is a conditional
//! `UPDATE` whose affected-row count is the verdict.

use ranch_domain::errors::RanchError;
use sqlx::SqlitePool;

/// Actor: `approved → consumed`, exactly once. `false` = this id authorizes
/// nothing (unknown, pending, denied, expired, already consumed, or approved
/// for another connection or another tool).
pub async fn consume(
    pool: &SqlitePool,
    id: &str,
    connection_id: &str,
    tool: &str,
) -> Result<bool, RanchError> {
    let result = sqlx::query(
        r#"
        UPDATE mcp_pending_approvals
           SET status = 'consumed'
         WHERE id = ? AND status = 'approved' AND connection_id = ? AND tool = ?
        "#,
    )
    .bind(id)
    .bind(connection_id)
    .bind(tool)
    .execute(pool)
    .await
    .map_err(RanchError::database)?;
    Ok(result.rows_affected() == 1)
}
