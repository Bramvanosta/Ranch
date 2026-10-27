//! MCP activity log, GUI side. `ranch-mcp` writes the
//! log; these commands read, export and clear it. UI-only: an agent must
//! never be able to read other agents' sessions through Ranch, nor erase its
//! own trail (`clear_mcp_activity` is on the MCP deny-list).

use crate::app_state::AppState;
use crate::core::errors::RanchErrorDto;
use crate::core::mcp::activity::{self, ActivityEvent, ActivityFilter, ActivitySession};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListMcpActivityArgs {
    pub filter: ActivityFilter,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpActivityPage {
    pub events: Vec<ActivityEvent>,
    /// Events matching the filter, across all pages.
    pub total: u32,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_mcp_activity(
    args: ListMcpActivityArgs,
    state: tauri::State<'_, AppState>,
) -> Result<McpActivityPage, RanchErrorDto> {
    let pool = state.db.pool();
    let events = activity::list(pool, &args.filter, args.limit, args.offset)
        .await
        .map_err(RanchErrorDto::from)?;
    let total = activity::count(pool, &args.filter)
        .await
        .map_err(RanchErrorDto::from)?;
    Ok(McpActivityPage { events, total })
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_mcp_activity_sessions(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ActivitySession>, RanchErrorDto> {
    activity::list_sessions(state.db.pool(), 200)
        .await
        .map_err(Into::into)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ActivityExportFormat {
    Json,
    Csv,
}

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportMcpActivityArgs {
    pub filter: ActivityFilter,
    pub format: ActivityExportFormat,
}

/// Returns the document as text; the frontend saves it.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn export_mcp_activity(
    args: ExportMcpActivityArgs,
    state: tauri::State<'_, AppState>,
) -> Result<String, RanchErrorDto> {
    activity::export(
        state.db.pool(),
        &args.filter,
        args.format == ActivityExportFormat::Csv,
    )
    .await
    .map_err(Into::into)
}

/// Returns how many events were removed.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn clear_mcp_activity(state: tauri::State<'_, AppState>) -> Result<u32, RanchErrorDto> {
    activity::clear(state.db.pool())
        .await
        .map(|n| n.min(u32::MAX as u64) as u32)
        .map_err(Into::into)
}
