//! MCP installer Tauri commands.
//!
//! Three commands:
//!   - `install_mcp_for_claude_desktop`
//!   - `install_mcp_for_cursor`
//!   - `get_mcp_installer_status`
//!   - `get_mcp_manual_snippet`
//!
//! These are intentionally UI-only (not exposed via MCP) — an agent
//! should not be able to wire itself into another agent's config.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::core::errors::{RanchError, RanchErrorDto};
use crate::core::mcp_installer::{self, McpClient, McpInstallerStatus, McpManualSnippet};

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpInstallerOverview {
    pub claude_desktop: McpInstallerStatus,
    pub cursor: McpInstallerStatus,
    pub claude_code: McpInstallerStatus,
    pub windsurf: McpInstallerStatus,
    pub codex: McpInstallerStatus,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_mcp_installer_status() -> Result<McpInstallerOverview, RanchErrorDto> {
    Ok(McpInstallerOverview {
        claude_desktop: mcp_installer::read_status_for(McpClient::ClaudeDesktop),
        cursor: mcp_installer::read_status_for(McpClient::Cursor),
        claude_code: mcp_installer::read_status_for(McpClient::ClaudeCode),
        windsurf: mcp_installer::read_status_for(McpClient::Windsurf),
        codex: mcp_installer::read_status_for(McpClient::Codex),
    })
}

/// One install command for every client (the two per-client
/// commands above predate it and stay for compatibility).
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn install_mcp_for_client(
    client: McpClient,
) -> Result<McpInstallerStatus, RanchErrorDto> {
    // `ClientCli` installers spawn a process: keep it off the async runtime.
    tokio::task::spawn_blocking(move || mcp_installer::install_for(client))
        .await
        .map_err(|e| {
            RanchErrorDto::from(RanchError::InvalidInput {
                reason: format!("Installer task failed: {e}"),
            })
        })?
        .map_err(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn install_mcp_for_claude_desktop() -> Result<McpInstallerStatus, RanchErrorDto> {
    install_mcp_for_claude_desktop_inner().map_err(Into::into)
}

pub fn install_mcp_for_claude_desktop_inner() -> Result<McpInstallerStatus, RanchError> {
    mcp_installer::install_for(McpClient::ClaudeDesktop)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn install_mcp_for_cursor() -> Result<McpInstallerStatus, RanchErrorDto> {
    install_mcp_for_cursor_inner().map_err(Into::into)
}

pub fn install_mcp_for_cursor_inner() -> Result<McpInstallerStatus, RanchError> {
    mcp_installer::install_for(McpClient::Cursor)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_mcp_manual_snippet() -> Result<McpManualSnippet, RanchErrorDto> {
    mcp_installer::manual_snippet().map_err(Into::into)
}
