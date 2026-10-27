//! Top-level error type for Ranch backend operations.
//!
//! The variants cover the data layer, the keychain and the Redis side, plus
//! the [`RanchErrorDto`] boundary type that serves
//! as the specta-derived TypeScript-facing shape (so downstream commands
//! returning `Result<_, RanchErrorDto>` get clean, typed error info on the
//! frontend).

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RanchError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Database migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("Cannot reach the OS keychain: {reason}. See {doc_url} for setup instructions.")]
    KeychainUnavailable { reason: String, doc_url: String },

    /// The secret store refused or failed (the OS keychain on the desktop, the
    /// secrets file in the server). A message, not the `keyring` error type:
    /// the core must build without that crate (it is an optional dependency).
    #[error("Keychain operation failed: {0}")]
    Keychain(String),

    #[error("Cannot resolve or create the application data directory: {0}")]
    AppData(#[from] std::io::Error),

    /// Surfaced by `parse_redis_url` when the user pastes garbage
    /// into the URL field. The frontend renders `reason` inline.
    #[error("Invalid Redis URL: {reason}")]
    InvalidRedisUrl { reason: String },

    /// low-level `redis-rs` errors that bubble up unchanged.
    /// `test_connection` maps these into a structured `TestConnectionError`
    /// before returning to the frontend, but other commands
    /// may propagate them via `?` directly.
    #[error("Redis client error: {0}")]
    Redis(#[from] redis::RedisError),

    /// Surfaced when a command is asked to operate on a connection
    /// id that no longer exists (e.g. deleted out-of-band between sidebar
    /// fetch and ping).
    #[error("Connection not found: {id}")]
    ConnectionNotFound { id: String },

    /// Surfaced when `rename_environment` (or any future env-
    /// targeted command) is given an unknown environment id.
    #[error("Environment not found: {id}")]
    EnvironmentNotFound { id: String },

    /// A group-targeted command was given an unknown group id.
    #[error("Group not found: {id}")]
    GroupNotFound { id: String },

    /// `delete_group` without `cascade` on a group that still
    /// holds connections. The UI answers with the confirmation dialog that
    /// lists them, then retries with `cascade = true`.
    #[error("Group \"{name}\" still contains {connection_count} connection(s)")]
    GroupNotEmpty { name: String, connection_count: u32 },

    /// Surfaced when a command's input fails a validation check
    /// before the IO step (empty label, out-of-range int, etc.). The
    /// frontend's Zod schema catches these first; this is the backend's
    /// belt-and-braces guard for direct MCP callers / programmatic use.
    #[error("Invalid input: {reason}")]
    InvalidInput { reason: String },

    /// A job search / sort scan stopped because a newer one was
    /// requested on the same connection. Not a failure — the caller
    /// has already moved on; clients must not retry it.
    #[error("Superseded by a newer job query")]
    Superseded,

    /// BullMQ schema not recognized for any discovered queue
    /// on the connection. Carries the raw `version` field from the first
    /// queue's `bull:{q}:meta` hash so the UI can show the user-facing
    /// "BullMQ v7.0.0 is not yet supported — upgrade Ranch or open a
    /// compatibility issue."
    #[error("BullMQ schema not recognized (detected version: {detected_version:?})")]
    SchemaNotRecognized { detected_version: Option<String> },

    /// Surfaced by `get_job` when `HGETALL bull:{queue}:{id}`
    /// returns an empty hash. The job was either deleted between the
    /// list-jobs fetch and the drill-down click (common race), or the
    /// user deep-linked a stale URL. The UI renders an empty state with
    /// a "← Back to queue" link instead of retrying.
    #[error("Job not found: {queue}/{id}")]
    JobNotFound { queue: String, id: String },

    /// A destructive command was rejected because the
    /// connection is read-only (Locked) and the user hasn't unlocked
    /// for the session. The frontend renders the `unlock_hint` so the
    /// user knows the keystroke. MCP clients get the same DTO
    /// and can surface it to agents.
    #[error("Connection {connection_name} is locked: {unlock_hint}")]
    ConnectionLocked {
        connection_id: String,
        connection_name: String,
        unlock_hint: String,
    },

    /// An MCP write tool targeted a connection that is MCP
    /// read-only (its own flag, or the global default). Step 1 of the MCP
    /// safety gate. The message names the setting so the agent
    /// can tell the human exactly what to change.
    #[error(
        "Blocked by Ranch: connection \"{connection_name}\" is read-only for agents, so `{tool}` was not executed. A human can allow agent writes for this connection in Ranch › Agents › Setup & safety."
    )]
    BlockedReadonly {
        connection_name: String,
        tool: String,
    },

    /// The human denied the agent's request in Ranch.
    #[error("Denied by the user in Ranch: `{tool}` was not executed.{}", reason.as_deref().map(|r| format!(" Reason: {r}")).unwrap_or_default())]
    ApprovalDenied {
        tool: String,
        reason: Option<String>,
    },

    /// Nobody answered the approval request in time.
    #[error(
        "Approval timed out after {seconds}s: `{tool}` was not executed. The user has to approve agent writes in the Ranch app."
    )]
    ApprovalTimeout { tool: String, seconds: u32 },

    /// The Ranch app is not running (or closed while the request
    /// was waiting), so nobody can approve. Fails fast instead of waiting
    /// for the timeout.
    #[error(
        "The Ranch app is not running, so `{tool}` cannot be approved and was not executed. Ask the user to open Ranch, then try again."
    )]
    HostUnavailable { tool: String },

    /// AI features are off (Settings › AI features): every MCP
    /// tool, read or write, is refused.
    #[error(
        "AI features are turned off in Ranch, so `{tool}` was not executed. Only a human can turn them back on, in Ranch › Settings › AI features."
    )]
    AiDisabled { tool: String },

    /// The connection actor refused an approval grant: unknown,
    /// not approved, already used, or issued for another connection / tool.
    /// Never expected in normal operation.
    #[error("The approval for `{tool}` is not valid (unknown, expired, denied or already used); nothing was executed.")]
    ApprovalInvalid { tool: String },

    /// A single-job destructive op (retry / remove / promote)
    /// was rejected because the job is not in the required state. E.g.
    /// retry requires `failed`, promote requires `delayed`. The frontend
    /// disables the button for visual feedback, but the backend ALSO
    /// guards so MCP / programmatic callers can't bypass.
    #[error(
        "Cannot {action} job {job_id}: current state is {current_state}, required {required_state}"
    )]
    InvalidJobState {
        action: String,
        connection_id: String,
        queue_name: String,
        job_id: String,
        current_state: String,
        required_state: String,
    },

    /// A scheduler-targeted op (`pause_scheduler` /
    /// `resume_scheduler`) could not locate the scheduler key in the
    /// `:repeat` ZSET. Surfaces when the user races with a concurrent
    /// removal (BullMQ's spawner ran and re-armed the scheduler with
    /// a different key, or another client deleted it).
    #[error("Scheduler not found: {queue_name}/{scheduler_key}")]
    SchedulerNotFound {
        connection_id: String,
        queue_name: String,
        scheduler_key: String,
    },

    /// The bastion rejected the SSH credential. Either the
    /// password was wrong, the public key wasn't accepted, or no
    /// credential was supplied. Surfaced verbatim through a toast / the
    /// form's error switch (`kind: "sshAuthFailed"`).
    #[error("SSH authentication failed: {reason}")]
    SshAuthFailed { reason: String },

    /// couldn't read or parse the SSH key file. `path` is
    /// the path the user typed; `reason` carries the underlying IO or
    /// parse error from `tokio::fs` / russh-keys.
    #[error("SSH key file error at {path}: {reason}")]
    SshKeyFileError { path: String, reason: String },

    /// catch-all for the SSH tunnel session itself: bastion
    /// unreachable, host-key mismatch, channel-open refused, jump-host
    /// hop failure, etc. The `reason` carries the disambiguation.
    #[error("SSH tunnel failed: {reason}")]
    SshTunnelFailed { reason: String },
}

/// Boundary type for command return values. The frontend always receives this
/// shape (never the raw `RanchError`), so `RanchError` can grow without churning the TS surface.
///
/// Conversion is one-way (`RanchError → RanchErrorDto`) — frontend errors
/// are display-only, never round-tripped.
#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RanchErrorDto {
    /// Discriminator (camelCase, matches the `kind` field in the
    /// `TestConnectionError` enum convention so all error consumers can pattern
    /// match the same way).
    pub kind: String,
    /// Human-readable message, safe to display verbatim in the UI.
    pub message: String,
    /// When the error is a `keychainUnavailable`, this is the URL the UI links
    /// to so users can fix their setup. None for every other variant.
    /// Further optional structured fields can be added without breaking the
    /// wire format.
    pub doc_url: Option<String>,
}

impl From<RanchError> for RanchErrorDto {
    fn from(e: RanchError) -> Self {
        match e {
            RanchError::Database(inner) => RanchErrorDto {
                kind: "database".to_string(),
                message: inner.to_string(),
                doc_url: None,
            },
            RanchError::Migration(inner) => RanchErrorDto {
                kind: "migration".to_string(),
                message: inner.to_string(),
                doc_url: None,
            },
            RanchError::KeychainUnavailable { reason, doc_url } => RanchErrorDto {
                kind: "keychainUnavailable".to_string(),
                message: reason,
                doc_url: Some(doc_url),
            },
            RanchError::Keychain(inner) => RanchErrorDto {
                kind: "keychain".to_string(),
                message: inner,
                doc_url: None,
            },
            RanchError::AppData(source) => RanchErrorDto {
                kind: "appData".to_string(),
                message: source.to_string(),
                doc_url: None,
            },
            RanchError::InvalidRedisUrl { reason } => RanchErrorDto {
                kind: "invalidRedisUrl".to_string(),
                message: reason,
                doc_url: None,
            },
            RanchError::Redis(inner) => RanchErrorDto {
                kind: "redis".to_string(),
                message: inner.to_string(),
                doc_url: None,
            },
            RanchError::ConnectionNotFound { id } => RanchErrorDto {
                kind: "connectionNotFound".to_string(),
                message: format!("Connection not found: {id}"),
                doc_url: None,
            },
            RanchError::EnvironmentNotFound { id } => RanchErrorDto {
                kind: "environmentNotFound".to_string(),
                message: format!("Environment not found: {id}"),
                doc_url: None,
            },
            RanchError::GroupNotFound { id } => RanchErrorDto {
                kind: "groupNotFound".to_string(),
                message: format!("Group not found: {id}"),
                doc_url: None,
            },
            RanchError::GroupNotEmpty {
                name,
                connection_count,
            } => RanchErrorDto {
                kind: "groupNotEmpty".to_string(),
                message: format!(
                    "Group \"{name}\" still contains {connection_count} connection(s)"
                ),
                doc_url: None,
            },
            RanchError::InvalidInput { reason } => RanchErrorDto {
                kind: "invalidInput".to_string(),
                message: reason,
                doc_url: None,
            },
            RanchError::Superseded => RanchErrorDto {
                kind: "superseded".to_string(),
                message: "Superseded by a newer job query".to_string(),
                doc_url: None,
            },
            RanchError::SchemaNotRecognized { detected_version } => RanchErrorDto {
                kind: "schemaNotRecognized".to_string(),
                message: format!(
                    "BullMQ schema not recognized (detected version: {})",
                    detected_version.unwrap_or_else(|| "<missing>".to_string())
                ),
                doc_url: None,
            },
            RanchError::JobNotFound { queue, id } => RanchErrorDto {
                kind: "jobNotFound".to_string(),
                message: format!("Job not found: {queue}/{id}"),
                doc_url: None,
            },
            RanchError::ConnectionLocked {
                connection_name,
                unlock_hint,
                ..
            } => RanchErrorDto {
                kind: "connectionLocked".to_string(),
                message: format!("Connection {connection_name} is locked: {unlock_hint}"),
                doc_url: None,
            },
            e @ RanchError::AiDisabled { .. } => RanchErrorDto {
                kind: "aiDisabled".to_string(),
                message: e.to_string(),
                doc_url: None,
            },
            e @ (RanchError::ApprovalDenied { .. }
            | RanchError::ApprovalTimeout { .. }
            | RanchError::HostUnavailable { .. }
            | RanchError::ApprovalInvalid { .. }) => RanchErrorDto {
                kind: match &e {
                    RanchError::ApprovalDenied { .. } => "approvalDenied",
                    RanchError::ApprovalTimeout { .. } => "approvalTimeout",
                    RanchError::HostUnavailable { .. } => "hostUnavailable",
                    _ => "approvalInvalid",
                }
                .to_string(),
                message: e.to_string(),
                doc_url: None,
            },
            RanchError::BlockedReadonly {
                connection_name,
                tool,
            } => RanchErrorDto {
                kind: "blockedReadonly".to_string(),
                message: format!(
                    "Blocked by Ranch: connection \"{connection_name}\" is read-only for agents, so `{tool}` was not executed. A human can allow agent writes for this connection in Ranch › Agents › Setup & safety."
                ),
                doc_url: None,
            },
            RanchError::InvalidJobState {
                action,
                job_id,
                current_state,
                required_state,
                ..
            } => RanchErrorDto {
                kind: "invalidJobState".to_string(),
                message: format!(
                    "Cannot {action} job {job_id}: current state is {current_state}, required {required_state}"
                ),
                doc_url: None,
            },
            RanchError::SchedulerNotFound {
                queue_name,
                scheduler_key,
                ..
            } => RanchErrorDto {
                kind: "schedulerNotFound".to_string(),
                message: format!("Scheduler not found: {queue_name}/{scheduler_key}"),
                doc_url: None,
            },
            RanchError::SshAuthFailed { reason } => RanchErrorDto {
                kind: "sshAuthFailed".to_string(),
                message: format!("SSH authentication failed: {reason}"),
                doc_url: None,
            },
            RanchError::SshKeyFileError { path, reason } => RanchErrorDto {
                kind: "sshKeyFileError".to_string(),
                message: format!("SSH key file error at {path}: {reason}"),
                doc_url: None,
            },
            RanchError::SshTunnelFailed { reason } => RanchErrorDto {
                kind: "sshTunnelFailed".to_string(),
                message: format!("SSH tunnel failed: {reason}"),
                doc_url: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_not_found_maps_to_kind_with_descriptive_message() {
        let err = RanchError::JobNotFound {
            queue: "emails".into(),
            id: "9999".into(),
        };
        let dto: RanchErrorDto = err.into();
        assert_eq!(dto.kind, "jobNotFound");
        assert!(dto.message.contains("emails"));
        assert!(dto.message.contains("9999"));
        assert!(dto.doc_url.is_none());
    }

    /// `ConnectionLocked` maps to `kind: "connectionLocked"`
    /// and carries the connection name + unlock hint in the message so
    /// the frontend can render the keystroke without a separate field.
    #[test]
    fn connection_locked_maps_to_kind_with_hint_in_message() {
        let err = RanchError::ConnectionLocked {
            connection_id: "conn-x".into(),
            connection_name: "Acme · prod".into(),
            unlock_hint: "Press cmd-shift-U to unlock for this session.".into(),
        };
        let dto: RanchErrorDto = err.into();
        assert_eq!(dto.kind, "connectionLocked");
        assert!(dto.message.contains("Acme · prod"));
        assert!(dto.message.contains("cmd-shift-U"));
        assert!(dto.doc_url.is_none());
    }

    /// `InvalidJobState` carries the action verb + the
    /// current/required state pair in a message a UI toast can render
    /// verbatim. `kind: "invalidJobState"` lets the frontend branch
    /// on it specifically (e.g. inline button-disabled affordance).
    #[test]
    fn invalid_job_state_maps_to_kind_with_action_and_states() {
        let err = RanchError::InvalidJobState {
            action: "retry".into(),
            connection_id: "conn-x".into(),
            queue_name: "emails".into(),
            job_id: "42".into(),
            current_state: "waiting".into(),
            required_state: "failed".into(),
        };
        let dto: RanchErrorDto = err.into();
        assert_eq!(dto.kind, "invalidJobState");
        assert!(dto.message.contains("retry"));
        assert!(dto.message.contains("42"));
        assert!(dto.message.contains("waiting"));
        assert!(dto.message.contains("failed"));
        assert!(dto.doc_url.is_none());
    }

    /// `SchedulerNotFound` is the scheduler analog of
    /// `JobNotFound`. Mirrors that variant's DTO shape.
    #[test]
    fn scheduler_not_found_maps_to_kind_with_queue_and_key() {
        let err = RanchError::SchedulerNotFound {
            connection_id: "conn-x".into(),
            queue_name: "emails".into(),
            scheduler_key: "daily-report".into(),
        };
        let dto: RanchErrorDto = err.into();
        assert_eq!(dto.kind, "schedulerNotFound");
        assert!(dto.message.contains("emails"));
        assert!(dto.message.contains("daily-report"));
        assert!(dto.doc_url.is_none());
    }

    /// `SshAuthFailed` carries the bastion's rejection
    /// reason so the form's error toast reads naturally.
    #[test]
    fn ssh_auth_failed_maps_to_kind_with_reason() {
        let err = RanchError::SshAuthFailed {
            reason: "Bastion rejected password authentication".into(),
        };
        let dto: RanchErrorDto = err.into();
        assert_eq!(dto.kind, "sshAuthFailed");
        assert!(dto.message.contains("rejected password"));
        assert!(dto.doc_url.is_none());
    }

    /// `SshKeyFileError` carries both the path (so the
    /// user can fix the typo) and the underlying IO/parse reason.
    #[test]
    fn ssh_key_file_error_maps_to_kind_with_path() {
        let err = RanchError::SshKeyFileError {
            path: "~/.ssh/missing_key".into(),
            reason: "No such file or directory".into(),
        };
        let dto: RanchErrorDto = err.into();
        assert_eq!(dto.kind, "sshKeyFileError");
        assert!(dto.message.contains("~/.ssh/missing_key"));
        assert!(dto.message.contains("No such file"));
        assert!(dto.doc_url.is_none());
    }

    /// `SshTunnelFailed` is the catch-all for tunnel /
    /// hostkey / channel / jump-hop failures. Generic shape; the
    /// `reason` carries disambiguation.
    #[test]
    fn ssh_tunnel_failed_maps_to_kind_with_reason() {
        let err = RanchError::SshTunnelFailed {
            reason: "Host key fingerprint mismatch — expected …, got …".into(),
        };
        let dto: RanchErrorDto = err.into();
        assert_eq!(dto.kind, "sshTunnelFailed");
        assert!(dto.message.contains("fingerprint mismatch"));
        assert!(dto.doc_url.is_none());
    }
}
