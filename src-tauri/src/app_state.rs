//! Application state shared across Tauri commands.
//!
//! Stored in Tauri's managed-state slot via `app.manage(AppState { ... })`
//! during the setup hook (see `lib.rs`). Commands access via
//! `tauri::State<AppState>` and clone the inner `Arc`s cheaply.

use crate::core::connection_actor::ConnectionRegistry;
use crate::core::db::Db;
use crate::core::keychain::KeychainAdapter;
use std::sync::Arc;

// Some fields are not read everywhere yet — the Tauri commands consume
// them, resolving them via `tauri::State<AppState>`.
#[allow(dead_code)]
pub struct AppState {
    pub db: Arc<Db>,
    pub keychain: Arc<dyn KeychainAdapter>,
    /// per-connection actor registry. Lazy-spawns Tokio
    /// tasks on first command for each connection_id; shut down on
    /// `remove_connection` or app exit.
    pub connections: Arc<ConnectionRegistry>,
}
