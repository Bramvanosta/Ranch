use async_trait::async_trait;
use ranch_domain::errors::RanchError;

/// The human approvals agents wait for.
// `async_trait` marks the boxed future `must_use` on top of the `Result`.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait ApprovalStore: Send + Sync {
    /// Trade an approval for the right to run `tool` on `connection_id`,
    /// once. `false` when the approval is unknown, not approved, already
    /// used, or was given for another connection or tool.
    async fn consume(
        &self,
        approval_id: &str,
        connection_id: &str,
        tool: &str,
    ) -> Result<bool, RanchError>;
}
