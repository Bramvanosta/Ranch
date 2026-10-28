//! The agent surface as the UI reads and drives it: the MCP activity log,
//! the approvals a human gives or refuses, and installing the MCP server
//! into agent clients.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::core::mcp::{activity, approval};
use crate::core::mcp_installer as installer;

/// Outcome of a tool call as the log records it. The four middle values are
/// the gate's verdicts (`data.kind` of the MCP error).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityStatus {
    Success,
    Error,
    BlockedReadonly,
    Denied,
    Timeout,
    HostUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub id: String,
    pub session_id: String,
    pub timestamp: String,
    pub client_hint: Option<String>,
    pub tool: String,
    /// `"read"` or `"write"`.
    pub kind: String,
    pub connection_id: Option<String>,
    pub connection_name: Option<String>,
    /// JSON text of the (possibly compacted) arguments.
    pub target_json: Option<String>,
    pub status: ActivityStatus,
    pub duration_ms: u32,
    pub error: Option<String>,
    pub approval_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActivityFilter {
    /// RFC 3339, inclusive.
    pub from: Option<String>,
    pub to: Option<String>,
    pub client_hint: Option<String>,
    pub connection_id: Option<String>,
    /// `"read"` or `"write"`.
    pub kind: Option<String>,
    pub status: Option<ActivityStatus>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySession {
    pub session_id: String,
    pub client_hint: Option<String>,
    pub started_at: String,
    pub last_at: String,
    pub total: u32,
    pub writes: u32,
    /// Calls that did not succeed, whatever the reason.
    pub not_successful: u32,
}

/// Global approval mode: off, writes only, or all. Not per connection: the
/// per-connection dimension is MCP read-only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// Never ask. Writes fall through to the session lock.
    Off,
    /// Ask for every write tool. **Default.**
    WritesOnly,
    /// Ask for every tool, reads included.
    All,
}

/// A request as the GUI shows it.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    pub id: String,
    pub created_at: String,
    pub client_hint: Option<String>,
    pub connection_id: String,
    pub connection_name: String,
    pub group_name: String,
    pub environment_name: String,
    pub env_color: String,
    pub tool: String,
    /// `"read"` or `"write"`.
    pub kind: String,
    /// JSON text of the tool arguments.
    pub arguments_json: String,
    /// JSON text of the target preview, when one could be built.
    pub target_preview_json: Option<String>,
    /// Seconds after `created_at` at which the agent gives up.
    pub timeout_seconds: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalDecision {
    Approve,
    Deny,
}

/// Result of a GUI decision.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DecideOutcome {
    Decided,
    /// The request was no longer pending (already decided, expired, purged).
    /// A no-op by design: the UI just moves on.
    AlreadyDecided,
}

/// What the audit log says about the approval behind an agent
/// write: who asked, what was decided, when.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRecord {
    pub id: String,
    pub created_at: String,
    pub decided_at: Option<String>,
    /// `pending`, `approved`, `denied`, `expired` or `consumed` (approved, then used).
    pub status: String,
    pub client_hint: Option<String>,
    pub tool: String,
    pub reason: Option<String>,
    /// The exact arguments the agent sent (JSON text).
    pub arguments_json: String,
}

/// Agents › Approvals: the requests already decided (approved,
/// denied, expired, consumed), newest first, with their connection.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalHistoryItem {
    pub id: String,
    pub created_at: String,
    pub decided_at: Option<String>,
    /// `approved`, `denied`, `expired` or `consumed` (approved, then used).
    pub status: String,
    pub client_hint: Option<String>,
    pub tool: String,
    pub reason: Option<String>,
    pub connection_id: String,
    /// `None` when the connection was removed since.
    pub connection_name: Option<String>,
}

/// The known MCP-aware clients Ranch can auto-install for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum McpClient {
    ClaudeDesktop,
    Cursor,
    /// JSON merge, like Cursor.
    Windsurf,
    /// Registered through the `claude` CLI.
    ClaudeCode,
    /// Registered through the `codex` CLI.
    Codex,
}

/// How a client is wired up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum InstallerKind {
    /// Ranch merges `mcpServers.ranch` into the client's JSON config.
    JsonMerge,
    /// The client owns its config; Ranch asks the client's own CLI to
    /// register the server (`claude mcp add …`, `codex mcp add …`).
    ClientCli,
}

/// Per-client status returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpInstallerStatus {
    pub client: McpClient,
    pub config_path: String,
    /// `true` when the Ranch MCP entry is present in the client's config.
    pub installed: bool,
    /// `true` when the config dir / file exists (i.e. the client itself
    /// is installed). When `false`, install will create the dir + file.
    pub config_dir_exists: bool,
    /// Last-seen error encountered while reading the config (malformed
    /// JSON, permission denied, etc.). `None` for the happy path.
    pub error: Option<String>,
    /// How this client is installed.
    pub kind: InstallerKind,
    /// `ClientCli` only: was the client's CLI found? `None` for `JsonMerge`.
    pub cli_found: Option<bool>,
    /// `ClientCli` only: the exact command, to copy when the CLI was not
    /// found (or to run by hand).
    pub manual_command: Option<String>,
}

/// manual-config snippet for clients we don't auto-install.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpManualSnippet {
    pub binary_path: String,
    pub json_snippet: String,
}

impl From<activity::ActivityStatus> for ActivityStatus {
    fn from(v: activity::ActivityStatus) -> Self {
        match v {
            activity::ActivityStatus::Success => Self::Success,
            activity::ActivityStatus::Error => Self::Error,
            activity::ActivityStatus::BlockedReadonly => Self::BlockedReadonly,
            activity::ActivityStatus::Denied => Self::Denied,
            activity::ActivityStatus::Timeout => Self::Timeout,
            activity::ActivityStatus::HostUnavailable => Self::HostUnavailable,
        }
    }
}

impl From<ActivityStatus> for activity::ActivityStatus {
    fn from(v: ActivityStatus) -> Self {
        match v {
            ActivityStatus::Success => Self::Success,
            ActivityStatus::Error => Self::Error,
            ActivityStatus::BlockedReadonly => Self::BlockedReadonly,
            ActivityStatus::Denied => Self::Denied,
            ActivityStatus::Timeout => Self::Timeout,
            ActivityStatus::HostUnavailable => Self::HostUnavailable,
        }
    }
}

impl From<activity::ActivityEvent> for ActivityEvent {
    fn from(v: activity::ActivityEvent) -> Self {
        Self {
            id: v.id,
            session_id: v.session_id,
            timestamp: v.timestamp,
            client_hint: v.client_hint,
            tool: v.tool,
            kind: v.kind,
            connection_id: v.connection_id,
            connection_name: v.connection_name,
            target_json: v.target_json,
            status: v.status.into(),
            duration_ms: v.duration_ms,
            error: v.error,
            approval_id: v.approval_id,
        }
    }
}

impl From<ActivityFilter> for activity::ActivityFilter {
    fn from(v: ActivityFilter) -> Self {
        Self {
            from: v.from,
            to: v.to,
            client_hint: v.client_hint,
            connection_id: v.connection_id,
            kind: v.kind,
            status: v.status.map(Into::into),
            session_id: v.session_id,
        }
    }
}

impl From<activity::ActivitySession> for ActivitySession {
    fn from(v: activity::ActivitySession) -> Self {
        Self {
            session_id: v.session_id,
            client_hint: v.client_hint,
            started_at: v.started_at,
            last_at: v.last_at,
            total: v.total,
            writes: v.writes,
            not_successful: v.not_successful,
        }
    }
}

impl From<approval::ApprovalMode> for ApprovalMode {
    fn from(v: approval::ApprovalMode) -> Self {
        match v {
            approval::ApprovalMode::Off => Self::Off,
            approval::ApprovalMode::WritesOnly => Self::WritesOnly,
            approval::ApprovalMode::All => Self::All,
        }
    }
}

impl From<ApprovalMode> for approval::ApprovalMode {
    fn from(v: ApprovalMode) -> Self {
        match v {
            ApprovalMode::Off => Self::Off,
            ApprovalMode::WritesOnly => Self::WritesOnly,
            ApprovalMode::All => Self::All,
        }
    }
}

impl From<approval::PendingApproval> for PendingApproval {
    fn from(v: approval::PendingApproval) -> Self {
        Self {
            id: v.id,
            created_at: v.created_at,
            client_hint: v.client_hint,
            connection_id: v.connection_id,
            connection_name: v.connection_name,
            group_name: v.group_name,
            environment_name: v.environment_name,
            env_color: v.env_color,
            tool: v.tool,
            kind: v.kind,
            arguments_json: v.arguments_json,
            target_preview_json: v.target_preview_json,
            timeout_seconds: v.timeout_seconds,
        }
    }
}

impl From<ApprovalDecision> for approval::ApprovalDecision {
    fn from(v: ApprovalDecision) -> Self {
        match v {
            ApprovalDecision::Approve => Self::Approve,
            ApprovalDecision::Deny => Self::Deny,
        }
    }
}

impl From<approval::DecideOutcome> for DecideOutcome {
    fn from(v: approval::DecideOutcome) -> Self {
        match v {
            approval::DecideOutcome::Decided => Self::Decided,
            approval::DecideOutcome::AlreadyDecided => Self::AlreadyDecided,
        }
    }
}

impl From<approval::ApprovalRecord> for ApprovalRecord {
    fn from(v: approval::ApprovalRecord) -> Self {
        Self {
            id: v.id,
            created_at: v.created_at,
            decided_at: v.decided_at,
            status: v.status,
            client_hint: v.client_hint,
            tool: v.tool,
            reason: v.reason,
            arguments_json: v.arguments_json,
        }
    }
}

impl From<approval::ApprovalHistoryItem> for ApprovalHistoryItem {
    fn from(v: approval::ApprovalHistoryItem) -> Self {
        Self {
            id: v.id,
            created_at: v.created_at,
            decided_at: v.decided_at,
            status: v.status,
            client_hint: v.client_hint,
            tool: v.tool,
            reason: v.reason,
            connection_id: v.connection_id,
            connection_name: v.connection_name,
        }
    }
}

impl From<installer::McpClient> for McpClient {
    fn from(v: installer::McpClient) -> Self {
        match v {
            installer::McpClient::ClaudeDesktop => Self::ClaudeDesktop,
            installer::McpClient::Cursor => Self::Cursor,
            installer::McpClient::Windsurf => Self::Windsurf,
            installer::McpClient::ClaudeCode => Self::ClaudeCode,
            installer::McpClient::Codex => Self::Codex,
        }
    }
}

impl From<McpClient> for installer::McpClient {
    fn from(v: McpClient) -> Self {
        match v {
            McpClient::ClaudeDesktop => Self::ClaudeDesktop,
            McpClient::Cursor => Self::Cursor,
            McpClient::Windsurf => Self::Windsurf,
            McpClient::ClaudeCode => Self::ClaudeCode,
            McpClient::Codex => Self::Codex,
        }
    }
}

impl From<installer::InstallerKind> for InstallerKind {
    fn from(v: installer::InstallerKind) -> Self {
        match v {
            installer::InstallerKind::JsonMerge => Self::JsonMerge,
            installer::InstallerKind::ClientCli => Self::ClientCli,
        }
    }
}

impl From<installer::McpInstallerStatus> for McpInstallerStatus {
    fn from(v: installer::McpInstallerStatus) -> Self {
        Self {
            client: v.client.into(),
            config_path: v.config_path,
            installed: v.installed,
            config_dir_exists: v.config_dir_exists,
            error: v.error,
            kind: v.kind.into(),
            cli_found: v.cli_found,
            manual_command: v.manual_command,
        }
    }
}

impl From<installer::McpManualSnippet> for McpManualSnippet {
    fn from(v: installer::McpManualSnippet) -> Self {
        Self {
            binary_path: v.binary_path,
            json_snippet: v.json_snippet,
        }
    }
}
