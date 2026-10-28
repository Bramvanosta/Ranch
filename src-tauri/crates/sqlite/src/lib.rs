//! The ports kept in `ranch.db`, and the database itself: the pool and its
//! migrations ([`db`]), the audit writer ([`audit`]), the trade of an
//! approval ([`approvals`]).
//!
//! A crate of its own: it reaches the rest of Ranch through the
//! application's ports alone, and its manifest names no queue store, shell or
//! wire format. The app crate re-exports it as `adapters::sqlite`.

// The stores answer `RanchError`; see the domain crate's note on
// `result_large_err`.
#![allow(clippy::result_large_err)]

pub mod approvals;
pub mod audit;
pub mod connections;
pub mod db;
pub mod settings;

pub use connections::SqliteConnectionRepository;
pub use db::Db;
pub use settings::SqliteSettingsStore;

use std::sync::Arc;

use async_trait::async_trait;
use ranch_application::ports::{ApprovalStore, AuditLog};
use ranch_application::WritePipeline;
use ranch_domain::audit::AuditEntry;
use ranch_domain::errors::RanchError;
use sqlx::SqlitePool;

pub struct SqliteAuditLog(pub SqlitePool);

#[async_trait]
impl AuditLog for SqliteAuditLog {
    async fn append(&self, entry: AuditEntry) -> Result<(), RanchError> {
        audit::record(&self.0, entry).await
    }
}

pub struct SqliteApprovalStore(pub SqlitePool);

#[async_trait]
impl ApprovalStore for SqliteApprovalStore {
    async fn consume(
        &self,
        approval_id: &str,
        connection_id: &str,
        tool: &str,
    ) -> Result<bool, RanchError> {
        approvals::consume(&self.0, approval_id, connection_id, tool).await
    }
}

/// The write pipeline over `ranch.db`.
pub fn write_pipeline(pool: &SqlitePool) -> Arc<WritePipeline> {
    Arc::new(WritePipeline::new(
        Arc::new(SqliteAuditLog(pool.clone())),
        Arc::new(SqliteApprovalStore(pool.clone())),
    ))
}
