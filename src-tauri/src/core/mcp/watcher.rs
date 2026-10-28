//! GUI-side background tasks for the approval queue.
//!
//! `ranch-mcp` cannot call into the GUI (it is another process) so the GUI
//! watches the shared table. Polling every 500 ms is one indexed query on a
//! table that is almost always empty; the frontend is only told when the set
//! of pending requests actually changes.

use std::sync::Arc;
use std::time::Duration;

use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};

use crate::application::ports::SettingsStore;
use crate::core::mcp::approval::{list_pending, purge_old};

/// Tauri event carrying the full list of pending approvals (`PendingApproval[]`).
pub const PENDING_APPROVAL_EVENT: &str = "mcp:pending_approval";
const WATCH_INTERVAL: Duration = Duration::from_millis(500);
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

/// Emit [`PENDING_APPROVAL_EVENT`] whenever the pending set changes
/// (a request arrived, was decided, expired or was purged).
pub fn spawn_approval_watcher(
    app: AppHandle,
    pool: SqlitePool,
    settings: Arc<dyn SettingsStore>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_ids: Vec<String> = Vec::new();
        let mut ticker = tokio::time::interval(WATCH_INTERVAL);
        loop {
            ticker.tick().await;
            match list_pending(&pool, settings.as_ref()).await {
                Ok(pending) => {
                    let ids: Vec<String> = pending.iter().map(|p| p.id.clone()).collect();
                    if ids != last_ids {
                        // The window receives the contract's shape.
                        let pending: Vec<crate::contract::mcp::PendingApproval> =
                            pending.into_iter().map(Into::into).collect();
                        if let Err(e) = app.emit(PENDING_APPROVAL_EVENT, &pending) {
                            tracing::warn!(error = %e, "could not emit mcp:pending_approval");
                        }
                        last_ids = ids;
                    }
                }
                Err(e) => tracing::warn!(error = %e, "could not read pending approvals"),
            }
        }
    })
}

/// Requests never pile up: purge anything older than an hour.
pub fn spawn_approval_cleanup(pool: SqlitePool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(CLEANUP_INTERVAL);
        loop {
            ticker.tick().await;
            match purge_old(&pool).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(purged = n, "purged old MCP approval requests"),
                Err(e) => tracing::warn!(error = %e, "could not purge MCP approval requests"),
            }
        }
    })
}
