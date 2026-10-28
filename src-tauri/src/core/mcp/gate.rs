//! The MCP safety gate: every MCP **write** tool goes through
//! here before it reaches its operation. Reads are free.
//!
//! Evaluation order:
//!   1. **MCP read-only**: `connections.mcp_readonly`,
//!      falling back to the global default. Blocks with `BlockedReadonly`.
//!   2. **Human approval**.
//!   3. **Lock**: a write that needs no approval is refused on a connection
//!      the app sealed as locked, as it stands now (the actor holds the lock
//!      it was opened with); the write pipeline checks the actor's lock
//!      too. An approval passes the lock, there as here.
//!
//! The model is deliberately a per-connection read-only flag and a *global*
//! approval mode.

use sqlx::SqlitePool;

use crate::application::catalogue::Kind;
use crate::application::ports::sealer::{Check, Sealer};
use crate::application::ports::settings::{SafetySetting, Sealed};
use crate::application::ports::{ConnectionRepository, SettingsStore};
use crate::domain::audit::ApprovalGrant;
use crate::domain::errors::RanchError;
use crate::domain::lock::{check_lock_or_err, ReadOnlyState};

/// `kv_settings` key of the AI features switch (Settings › AI
/// features). Unset = on: existing installs keep their agents.
pub const AI_ENABLED_KEY: &str = SafetySetting::AiEnabled.key();

/// Whether AI features are on. Only an explicit `false` turns them off; a
/// read error counts as off, so a broken database never lets an agent in.
pub async fn ai_enabled(settings: &dyn SettingsStore) -> bool {
    match settings.safety(SafetySetting::AiEnabled).await {
        // Someone else wrote it: off. A keychain that cannot be read leaves
        // the value as it is (writes stay blocked, see `effective_mcp_readonly`).
        Ok(Sealed {
            check: Check::Invalid,
            ..
        }) => false,
        Ok(Sealed { value, .. }) => value.as_deref() != Some("false"),
        Err(_) => false,
    }
}

/// `kv_settings` key of the global "MCP read-only by default" switch.
pub const MCP_READONLY_DEFAULT_KEY: &str = SafetySetting::ReadonlyDefault.key();

/// Static classification of an MCP tool: its operation's kind in the
/// catalogue (`application::catalogue`). There is no runtime heuristic: a
/// tool is a read or a write by declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Read,
    Write,
}

impl From<Kind> for ToolKind {
    fn from(kind: Kind) -> Self {
        if kind.is_write() {
            Self::Write
        } else {
            Self::Read
        }
    }
}

/// The global default applied to connections without an explicit setting.
/// `false` when unset (or unreadable garbage; fail open here is fine: the
/// per-connection flag and the approval step are the real guards, and
/// Production connections carry an explicit `1`).
pub async fn mcp_readonly_default(settings: &dyn SettingsStore) -> Result<bool, RanchError> {
    let Sealed { value, check } = settings.safety(SafetySetting::ReadonlyDefault).await?;
    if check != Check::Valid {
        return Ok(true);
    }
    Ok(value.map(|v| v == "true").unwrap_or(false))
}

/// `COALESCE(connections.mcp_readonly, global default)`, plus the connection
/// name for error messages.
pub async fn effective_mcp_readonly(
    settings: &dyn SettingsStore,
    connections: &dyn ConnectionRepository,
    connection_id: &str,
) -> Result<(bool, String), RanchError> {
    let access = connections
        .agent_access(connection_id)
        .await?
        .ok_or_else(|| RanchError::ConnectionNotFound {
            id: connection_id.to_string(),
        })?;
    // The flag (and the lock the actor reads) as the app sealed them, or
    // read-only: a value another process wrote opens nothing.
    if !access.sealed {
        return Ok((true, access.name));
    }
    let effective = match access.mcp_readonly {
        Some(v) => v,
        None => mcp_readonly_default(settings).await?,
    };
    Ok((effective, access.name))
}

/// Gate step 1. `Ok(())` when `tool` may proceed on `connection_id`.
pub async fn check_mcp_readonly(
    settings: &dyn SettingsStore,
    connections: &dyn ConnectionRepository,
    connection_id: &str,
    tool: &str,
    kind: ToolKind,
) -> Result<(), RanchError> {
    if kind == ToolKind::Read {
        return Ok(());
    }
    let (readonly, connection_name) =
        effective_mcp_readonly(settings, connections, connection_id).await?;
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

/// Gate step 3 for a write that needs no approval: the lock the app sealed,
/// read now. A row whose seal does not hold was blocked as read-only at
/// step 1.
pub async fn check_sealed_lock(
    connections: &dyn ConnectionRepository,
    connection_id: &str,
) -> Result<(), RanchError> {
    let access = connections
        .agent_access(connection_id)
        .await?
        .ok_or_else(|| RanchError::ConnectionNotFound {
            id: connection_id.to_string(),
        })?;
    let lock = if access.locked || !access.sealed {
        ReadOnlyState::Locked
    } else {
        ReadOnlyState::Unlocked
    };
    check_lock_or_err(lock, connection_id, &access.name)
}

/// Steps 1 and 2a of the gate, before anything costly: MCP read-only, then
/// "is an approval needed, and is someone there to give it?". `Ok(false)` =
/// proceed under the lock (checked here as sealed, then by the actor);
/// `Ok(true)` = build the target preview and call [`request_approval`].
pub async fn precheck(
    settings: &dyn SettingsStore,
    connections: &dyn ConnectionRepository,
    tool: &str,
    kind: ToolKind,
    connection_id: &str,
) -> Result<bool, RanchError> {
    check_mcp_readonly(settings, connections, connection_id, tool, kind).await?;
    if !approval::approval_mode(settings)
        .await?
        .requires_approval(kind)
    {
        if kind == ToolKind::Write {
            check_sealed_lock(connections, connection_id).await?;
        }
        return Ok(false);
    }
    // Fail fast: never queue a request nobody can see.
    if !is_gui_alive(settings).await? {
        return Err(RanchError::HostUnavailable {
            tool: tool.to_string(),
        });
    }
    Ok(true)
}

/// Step 2b: queue the request and wait for the human. `pool` holds the
/// approval queue; `sealer` checks the app's seal on the answer.
pub async fn request_approval(
    pool: &SqlitePool,
    settings: &dyn SettingsStore,
    sealer: &dyn Sealer,
    request: GateRequest,
) -> Result<ApprovalGrant, RanchError> {
    let timeout = approval::approval_timeout(settings).await?;
    let new = NewApproval {
        client_hint: request.client_hint,
        connection_id: request.connection_id,
        tool: request.tool.clone(),
        kind: request.kind,
        arguments: request.arguments,
        target_preview: request.target_preview,
    };
    let id = approval::enqueue(pool, settings, &new).await?;

    let interval = approval::APPROVAL_POLL_INTERVAL;
    match approval::poll_decision(pool, settings, &id, timeout, interval).await? {
        // "approved" alone is a word in a file any process can write: only
        // the app's seal over this very call counts.
        PollOutcome::Approved if !approval::sealed_by_the_app(pool, sealer, &id, &new).await? => {
            Err(RanchError::ApprovalDenied {
                tool: request.tool,
                reason: Some("Ranch could not verify this approval; nothing was done".to_string()),
            })
        }
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
    settings: &dyn SettingsStore,
    connections: &dyn ConnectionRepository,
    sealer: &dyn Sealer,
    request: GateRequest,
) -> Result<Option<ApprovalGrant>, RanchError> {
    if !precheck(
        settings,
        connections,
        &request.tool,
        request.kind,
        &request.connection_id,
    )
    .await?
    {
        return Ok(None);
    }
    request_approval(pool, settings, sealer, request)
        .await
        .map(Some)
}
