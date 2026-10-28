use async_trait::async_trait;
use ranch_domain::audit::AuditEntry;
use ranch_domain::errors::RanchError;

/// Where every write's outcome is kept. `append` returns once the entry
/// is durable: a write is not reported as done before it is recorded.
// `async_trait` marks the boxed future `must_use` on top of the `Result`.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait AuditLog: Send + Sync {
    async fn append(&self, entry: AuditEntry) -> Result<(), RanchError>;
}
