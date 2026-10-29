//! An error as every shell sends it: the Tauri commands, the HTTP routes
//! and the MCP tools.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::domain::errors::RanchError;

/// Boundary type for command return values. The frontend always receives this
/// shape (never the raw `RanchError`), so `RanchError` can grow without churning the TS surface.
///
/// Conversion is one-way (`RanchError → RanchErrorDto`); frontend errors
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
            // Both keep the `invalidInput` kind that agents and the UI know.
            e @ (RanchError::Cancelled | RanchError::BackendUnavailable { .. }) => RanchErrorDto {
                kind: "invalidInput".to_string(),
                message: e.to_string(),
                doc_url: None,
            },
            e @ RanchError::Unsupported { .. } => RanchErrorDto {
                kind: "unsupported".to_string(),
                message: e.to_string(),
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
            e @ RanchError::ConnectionUnverified { .. } => RanchErrorDto {
                kind: "connectionUnverified".to_string(),
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
    use crate::adapters::bullmq_redis::bullmq_schema::JobState;

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

    #[test]
    fn connection_unverified_maps_to_its_kind_and_names_the_connection() {
        let dto: RanchErrorDto = RanchError::ConnectionUnverified {
            connection_name: "lumen-api prod".into(),
        }
        .into();
        assert_eq!(dto.kind, "connectionUnverified");
        assert!(dto.message.contains("\"lumen-api prod\""));
        assert!(dto.doc_url.is_none());
    }

    /// Cancellation and a vanished actor are their own variants inside
    /// Ranch, but keep the `invalidInput` kind and message on the wire.
    #[test]
    fn cancelled_and_backend_unavailable_keep_their_wire_shape() {
        let dto: RanchErrorDto = RanchError::Cancelled.into();
        assert_eq!(
            (dto.kind.as_str(), dto.message.as_str()),
            ("invalidInput", "Operation cancelled")
        );
        let dto: RanchErrorDto = RanchError::BackendUnavailable {
            reason: "Connection actor unavailable (mailbox closed)".into(),
        }
        .into();
        assert_eq!(dto.kind, "invalidInput");
        assert_eq!(
            dto.message,
            "Connection actor error: Connection actor unavailable (mailbox closed)"
        );
    }

    #[test]
    fn unsupported_maps_to_its_kind_and_names_the_capability() {
        let dto: RanchErrorDto = RanchError::Unsupported {
            capability: "schedules.pause".into(),
        }
        .into();
        assert_eq!(dto.kind, "unsupported");
        assert_eq!(
            dto.message,
            "This connection's queue backend does not support schedules.pause."
        );
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
            current_state: JobState::Waiting,
            required_state: JobState::Failed,
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
            reason: "Host key fingerprint mismatch: expected …, got …".into(),
        };
        let dto: RanchErrorDto = err.into();
        assert_eq!(dto.kind, "sshTunnelFailed");
        assert!(dto.message.contains("fingerprint mismatch"));
        assert!(dto.doc_url.is_none());
    }
}
