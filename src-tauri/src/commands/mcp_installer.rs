//! MCP installer Tauri commands.
//!
//! Three commands:
//!   - `install_mcp_for_claude_desktop`
//!   - `install_mcp_for_cursor`
//!   - `get_mcp_installer_status`
//!   - `get_mcp_manual_snippet`
//!
//! These are intentionally UI-only (not exposed via MCP): an agent
//! should not be able to wire itself into another agent's config.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::contract::errors::RanchErrorDto;

use crate::core::mcp_installer;
use crate::domain::errors::RanchError;

pub use crate::contract::mcp::{McpClient, McpInstallerStatus, McpManualSnippet};

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpInstallerOverview {
    pub claude_desktop: McpInstallerStatus,
    pub cursor: McpInstallerStatus,
    pub claude_code: McpInstallerStatus,
    pub windsurf: McpInstallerStatus,
    pub codex: McpInstallerStatus,
}

/// One client's status, as the UI reads it.
#[cfg(feature = "desktop")]
fn status(client: mcp_installer::McpClient) -> McpInstallerStatus {
    mcp_installer::read_status_for(client).into()
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_mcp_installer_status() -> Result<McpInstallerOverview, RanchErrorDto> {
    Ok(McpInstallerOverview {
        claude_desktop: status(mcp_installer::McpClient::ClaudeDesktop),
        cursor: status(mcp_installer::McpClient::Cursor),
        claude_code: status(mcp_installer::McpClient::ClaudeCode),
        windsurf: status(mcp_installer::McpClient::Windsurf),
        codex: status(mcp_installer::McpClient::Codex),
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
    let client = client.into();
    tokio::task::spawn_blocking(move || mcp_installer::install_for(client))
        .await
        .map_err(|e| {
            RanchErrorDto::from(RanchError::InvalidInput {
                reason: format!("Installer task failed: {e}"),
            })
        })?
        .map(Into::into)
        .map_err(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn install_mcp_for_claude_desktop() -> Result<McpInstallerStatus, RanchErrorDto> {
    install_mcp_for_claude_desktop_inner().map_err(Into::into)
}

pub fn install_mcp_for_claude_desktop_inner() -> Result<McpInstallerStatus, RanchError> {
    mcp_installer::install_for(mcp_installer::McpClient::ClaudeDesktop).map(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn install_mcp_for_cursor() -> Result<McpInstallerStatus, RanchErrorDto> {
    install_mcp_for_cursor_inner().map_err(Into::into)
}

pub fn install_mcp_for_cursor_inner() -> Result<McpInstallerStatus, RanchError> {
    mcp_installer::install_for(mcp_installer::McpClient::Cursor).map(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_mcp_manual_snippet() -> Result<McpManualSnippet, RanchErrorDto> {
    mcp_installer::manual_snippet()
        .map(Into::into)
        .map_err(Into::into)
}
