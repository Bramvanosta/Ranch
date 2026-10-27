//! MCP safety settings.
//!
//! Everything in this module is **UI-only**: these commands loosen or
//! tighten what agents may do, so an agent must never be able to call them.
//! They are on `registry::MCP_FORBIDDEN_COMMANDS` and the parity test fails
//! if one ever shows up on the MCP surface.

use crate::app_state::AppState;
use crate::commands::settings::set_kv_setting_inner;
use crate::core::errors::{RanchError, RanchErrorDto};
use crate::core::mcp::approval::{
    self, approval_mode, approval_timeout, ApprovalDecision, ApprovalMode, DecideOutcome,
    PendingApproval, APPROVAL_MODE_KEY, APPROVAL_TIMEOUT_KEY, MAX_APPROVAL_TIMEOUT_SECONDS,
    MIN_APPROVAL_TIMEOUT_SECONDS,
};
use crate::core::mcp::gate::{
    ai_enabled, mcp_readonly_default, AI_ENABLED_KEY, MCP_READONLY_DEFAULT_KEY,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::SqlitePool;

/// Global MCP safety settings (Agents › Setup & safety).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpSafetySettings {
    /// When true, every connection without an explicit setting is read-only
    /// for agents. Default: false.
    pub readonly_default: bool,
    /// Which agent calls wait for a human decision.
    /// Default: `writes_only`.
    pub approval_mode: ApprovalMode,
    /// How long an agent waits for that decision.
    /// Default 120, clamped to 10–3600.
    pub approval_timeout_seconds: u32,
    /// Bring Ranch to the front and keep it on top while an
    /// approval is pending. Default: true.
    pub approval_always_on_top: bool,
    /// Native notification + short sound when a request arrives
    ///. Default: true.
    pub approval_notify_sound: bool,
}

const APPROVAL_ALWAYS_ON_TOP_KEY: &str = "mcp_approval_always_on_top";
const APPROVAL_NOTIFY_SOUND_KEY: &str = "mcp_approval_notify_sound";

/// Boolean kv setting that defaults to `true` (only an explicit "false" is off).
async fn kv_bool_default_true(pool: &SqlitePool, key: &str) -> Result<bool, RanchError> {
    Ok(crate::commands::settings::get_kv_setting_inner(pool, key)
        .await?
        .map(|v| v != "false")
        .unwrap_or(true))
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_mcp_safety_settings(
    state: tauri::State<'_, AppState>,
) -> Result<McpSafetySettings, RanchErrorDto> {
    get_mcp_safety_settings_inner(state.db.pool())
        .await
        .map_err(Into::into)
}

pub async fn get_mcp_safety_settings_inner(
    pool: &SqlitePool,
) -> Result<McpSafetySettings, RanchError> {
    Ok(McpSafetySettings {
        readonly_default: mcp_readonly_default(pool).await?,
        approval_mode: approval_mode(pool).await?,
        approval_timeout_seconds: approval_timeout(pool).await?.as_secs() as u32,
        approval_always_on_top: kv_bool_default_true(pool, APPROVAL_ALWAYS_ON_TOP_KEY).await?,
        approval_notify_sound: kv_bool_default_true(pool, APPROVAL_NOTIFY_SOUND_KEY).await?,
    })
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn set_mcp_safety_settings(
    settings: McpSafetySettings,
    state: tauri::State<'_, AppState>,
) -> Result<McpSafetySettings, RanchErrorDto> {
    set_mcp_safety_settings_inner(&settings, state.db.pool())
        .await
        .map_err(Into::into)
}

pub async fn set_mcp_safety_settings_inner(
    settings: &McpSafetySettings,
    pool: &SqlitePool,
) -> Result<McpSafetySettings, RanchError> {
    set_kv_setting_inner(
        pool,
        MCP_READONLY_DEFAULT_KEY,
        if settings.readonly_default {
            "true"
        } else {
            "false"
        },
    )
    .await?;
    set_kv_setting_inner(pool, APPROVAL_MODE_KEY, settings.approval_mode.as_str()).await?;
    let timeout = settings
        .approval_timeout_seconds
        .clamp(MIN_APPROVAL_TIMEOUT_SECONDS, MAX_APPROVAL_TIMEOUT_SECONDS);
    set_kv_setting_inner(pool, APPROVAL_TIMEOUT_KEY, &timeout.to_string()).await?;
    for (key, value) in [
        (APPROVAL_ALWAYS_ON_TOP_KEY, settings.approval_always_on_top),
        (APPROVAL_NOTIFY_SOUND_KEY, settings.approval_notify_sound),
    ] {
        set_kv_setting_inner(pool, key, if value { "true" } else { "false" }).await?;
    }
    get_mcp_safety_settings_inner(pool).await
}

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetConnectionMcpReadonlyArgs {
    pub connection_id: String,
    /// `Some(true)` = read-only for agents, `Some(false)` = agents may write
    /// (subject to approval), `None` = follow the global default.
    pub mcp_readonly: Option<bool>,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn set_connection_mcp_readonly(
    args: SetConnectionMcpReadonlyArgs,
    state: tauri::State<'_, AppState>,
) -> Result<(), RanchErrorDto> {
    set_connection_mcp_readonly_inner(&args.connection_id, args.mcp_readonly, state.db.pool())
        .await
        .map_err(Into::into)
}

pub async fn set_connection_mcp_readonly_inner(
    connection_id: &str,
    mcp_readonly: Option<bool>,
    pool: &SqlitePool,
) -> Result<(), RanchError> {
    let now = chrono::Utc::now().to_rfc3339();
    let result =
        sqlx::query("UPDATE connections SET mcp_readonly = ?, updated_at = ? WHERE id = ?")
            .bind(mcp_readonly.map(|v| if v { 1_i64 } else { 0_i64 }))
            .bind(&now)
            .bind(connection_id)
            .execute(pool)
            .await?;
    if result.rows_affected() == 0 {
        return Err(RanchError::ConnectionNotFound {
            id: connection_id.to_string(),
        });
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// The approval queue, GUI side
// ─────────────────────────────────────────────────────────────────────────────

/// Requests waiting for a decision, oldest first.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_pending_approvals(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<PendingApproval>, RanchErrorDto> {
    approval::list_pending(state.db.pool())
        .await
        .map_err(Into::into)
}

/// The approval behind an audited agent write (who asked, what
/// was decided, when); `None` once purged.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_approval_record(
    approval_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Option<approval::ApprovalRecord>, RanchErrorDto> {
    approval::record(state.db.pool(), &approval_id)
        .await
        .map_err(Into::into)
}

/// Agents › Approvals, earlier decisions (UI-only,
/// MCP-forbidden: an agent must not read other agents' requests).
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_approval_history(
    limit: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<approval::ApprovalHistoryItem>, RanchErrorDto> {
    approval::history(state.db.pool(), limit)
        .await
        .map_err(Into::into)
}

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DecidePendingApprovalArgs {
    pub approval_id: String,
    pub decision: ApprovalDecision,
    /// Optional, shown to the agent when denying.
    pub reason: Option<String>,
}

/// The human's decision. **The** command an agent must never reach: it is on
/// the MCP deny-list, and the only caller is the approval modal.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn decide_pending_approval(
    args: DecidePendingApprovalArgs,
    state: tauri::State<'_, AppState>,
) -> Result<DecideOutcome, RanchErrorDto> {
    approval::decide(
        state.db.pool(),
        &args.approval_id,
        args.decision,
        args.reason.as_deref(),
    )
    .await
    .map_err(Into::into)
}

/// Settings › AI features: on or off. Off, `ranch-mcp`
/// refuses every tool (`aiDisabled`) and the requests waiting for a human are
/// denied, so no agent keeps waiting; MCP client configs are left as they are.
/// UI-only and MCP-forbidden: an agent must never switch itself back on.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn set_ai_features(
    enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<bool, RanchErrorDto> {
    set_ai_features_inner(state.db.pool(), enabled)
        .await
        .map_err(Into::into)
}

pub async fn set_ai_features_inner(pool: &SqlitePool, enabled: bool) -> Result<bool, RanchError> {
    set_kv_setting_inner(pool, AI_ENABLED_KEY, if enabled { "true" } else { "false" }).await?;
    if !enabled {
        sqlx::query(
            "UPDATE mcp_pending_approvals SET status = 'denied', decided_at = ?, reason = ? WHERE status = 'pending'",
        )
        .bind(chrono::Utc::now().to_rfc3339())
        .bind("AI features were turned off in Ranch")
        .execute(pool)
        .await?;
    }
    Ok(ai_enabled(pool).await)
}
