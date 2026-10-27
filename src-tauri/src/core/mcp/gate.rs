//! The MCP safety gate — every MCP **write** tool goes through
//! here before anything reaches the connection actor. Reads are free.
//!
//! Evaluation order:
//!   1. **MCP read-only** — `connections.mcp_readonly`,
//!      falling back to the global default. Blocks with `BlockedReadonly`.
//!   2. **Human approval**.
//!   3. **Session lock** — unchanged, enforced inside the actor.
//!
//! The model is deliberately Tabularis's: a per-connection read-only flag and
//! a *global* approval mode.

use sqlx::SqlitePool;

use crate::commands::settings::get_kv_setting_inner;
use crate::core::errors::RanchError;

/// `kv_settings` key of the AI features switch (Settings › AI
/// features). Unset = on: existing installs keep their agents.
pub const AI_ENABLED_KEY: &str = "ai_enabled";

/// Whether AI features are on. Only an explicit `false` turns them off; a
/// read error counts as off, so a broken database never lets an agent in.
pub async fn ai_enabled(pool: &SqlitePool) -> bool {
    match get_kv_setting_inner(pool, AI_ENABLED_KEY).await {
        Ok(value) => value.as_deref() != Some("false"),
        Err(_) => false,
    }
}

/// `kv_settings` key of the global "MCP read-only by default" switch.
pub const MCP_READONLY_DEFAULT_KEY: &str = "mcp_readonly_default";

/// Static classification of an MCP tool (see `commands::registry::MCP_TOOLS`).
/// There is no runtime heuristic: a tool is a read or a write by declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Read,
    Write,
}

/// The global default applied to connections without an explicit setting.
/// `false` when unset (or unreadable garbage — fail open here is fine: the
/// per-connection flag and the approval step are the real guards, and
/// Production connections carry an explicit `1`).
pub async fn mcp_readonly_default(pool: &SqlitePool) -> Result<bool, RanchError> {
    Ok(get_kv_setting_inner(pool, MCP_READONLY_DEFAULT_KEY)
        .await?
        .map(|v| v == "true")
        .unwrap_or(false))
}

/// `COALESCE(connections.mcp_readonly, global default)`, plus the connection
/// name for error messages.
pub async fn effective_mcp_readonly(
    pool: &SqlitePool,
    connection_id: &str,
) -> Result<(bool, String), RanchError> {
    let row: Option<(Option<i64>, String)> =
        sqlx::query_as("SELECT mcp_readonly, name FROM connections WHERE id = ?")
            .bind(connection_id)
            .fetch_optional(pool)
            .await?;
    let (explicit, name) = row.ok_or_else(|| RanchError::ConnectionNotFound {
        id: connection_id.to_string(),
    })?;
    let effective = match explicit {
        Some(v) => v != 0,
        None => mcp_readonly_default(pool).await?,
    };
    Ok((effective, name))
}

/// Gate step 1. `Ok(())` when `tool` may proceed on `connection_id`.
pub async fn check_mcp_readonly(
    pool: &SqlitePool,
    connection_id: &str,
    tool: &str,
    kind: ToolKind,
) -> Result<(), RanchError> {
    if kind == ToolKind::Read {
        return Ok(());
    }
    let (readonly, connection_name) = effective_mcp_readonly(pool, connection_id).await?;
    if readonly {
        return Err(RanchError::BlockedReadonly {
            connection_name,
            tool: tool.to_string(),
        });
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Human approval
// ─────────────────────────────────────────────────────────────────────────────

use crate::core::mcp::approval::{self, NewApproval, PollOutcome};
use crate::core::mcp::heartbeat::is_gui_alive;

/// Proof that a human approved one specific tool call. The connection actor
/// trades it (once) for the right to execute even on a session-locked
/// connection — the approval *is* the explicit human gesture, scoped to a
/// single operation.
///
/// The fields are private and the only constructor is visible to
/// `core::mcp` alone: UI code cannot fabricate a grant, and neither can a
/// future contributor by accident. The actor still re-checks the id against
/// the database (`approval::consume`), so a grant is worthless without its
/// `approved` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalGrant {
    approval_id: String,
    tool: String,
}

impl ApprovalGrant {
    pub(in crate::core::mcp) fn new(approval_id: String, tool: String) -> Self {
        Self { approval_id, tool }
    }

    pub fn approval_id(&self) -> &str {
        &self.approval_id
    }

    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// Test seam for the approval-grant matrix (forged / stale ids). Production code
    /// has no path to this outside `core::mcp`.
    #[doc(hidden)]
    pub fn forge_for_test(approval_id: &str, tool: &str) -> Self {
        Self::new(approval_id.to_string(), tool.to_string())
    }
}

/// One tool call asking to pass the gate.
#[derive(Debug, Clone)]
pub struct GateRequest {
    pub tool: String,
    pub kind: ToolKind,
    pub connection_id: String,
    pub client_hint: Option<String>,
    pub arguments: serde_json::Value,
    pub target_preview: Option<serde_json::Value>,
}

/// Steps 1 and 2a of the gate, before anything costly: MCP read-only, then
/// "is an approval needed, and is someone there to give it?". `Ok(false)` =
/// proceed under the session lock; `Ok(true)` = build the target preview and
/// call [`request_approval`].
pub async fn precheck(
    pool: &SqlitePool,
    tool: &str,
    kind: ToolKind,
    connection_id: &str,
) -> Result<bool, RanchError> {
    check_mcp_readonly(pool, connection_id, tool, kind).await?;
    if !approval::approval_mode(pool).await?.requires_approval(kind) {
        return Ok(false);
    }
    // Fail fast: never queue a request nobody can see.
    if !is_gui_alive(pool).await? {
        return Err(RanchError::HostUnavailable {
            tool: tool.to_string(),
        });
    }
    Ok(true)
}

/// Step 2b: queue the request and wait for the human.
pub async fn request_approval(
    pool: &SqlitePool,
    request: GateRequest,
) -> Result<ApprovalGrant, RanchError> {
    let timeout = approval::approval_timeout(pool).await?;
    let id = approval::enqueue(
        pool,
        &NewApproval {
            client_hint: request.client_hint,
            connection_id: request.connection_id,
            tool: request.tool.clone(),
            kind: request.kind,
            arguments: request.arguments,
            target_preview: request.target_preview,
        },
    )
    .await?;

    match approval::poll_decision(pool, &id, timeout, approval::APPROVAL_POLL_INTERVAL).await? {
        PollOutcome::Approved => Ok(ApprovalGrant::new(id, request.tool)),
        PollOutcome::Denied { reason } => Err(RanchError::ApprovalDenied {
            tool: request.tool,
            reason,
        }),
        PollOutcome::TimedOut => Err(RanchError::ApprovalTimeout {
            tool: request.tool,
            seconds: timeout.as_secs() as u32,
        }),
        PollOutcome::HostUnavailable => Err(RanchError::HostUnavailable { tool: request.tool }),
    }
}

/// The whole gate in one call, for callers whose preview is cheap or absent.
/// `Ok(None)` = proceed under the session lock (approval not required);
/// `Ok(Some(grant))` = a human approved this call.
pub async fn authorize(
    pool: &SqlitePool,
    request: GateRequest,
) -> Result<Option<ApprovalGrant>, RanchError> {
    if !precheck(pool, &request.tool, request.kind, &request.connection_id).await? {
        return Ok(None);
    }
    request_approval(pool, request).await.map(Some)
}
