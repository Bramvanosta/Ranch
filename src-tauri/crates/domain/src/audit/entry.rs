//! Audit domain types: source, outcome, input (`AuditEntry`),
//! stored shape (`AuditLogEntry`).

use super::action::AuditAction;
use super::target::AuditTarget;

/// Who initiated the action: a human via the Ranch UI, or an agent
/// via the MCP server (the MCP server populates the agent path).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditSource {
    Ui,
    Agent,
}

impl AuditSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ui => "ui",
            Self::Agent => "agent",
        }
    }

    pub fn try_from_str(s: &str) -> Option<Self> {
        match s {
            "ui" => Some(Self::Ui),
            "agent" => Some(Self::Agent),
            _ => None,
        }
    }
}

/// Outcome of the action: did it succeed, fail, or cancel mid-flight?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOutcome {
    Success,
    Failure,
    Cancelled,
}

impl AuditOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn try_from_str(s: &str) -> Option<Self> {
        match s {
            "success" => Some(Self::Success),
            "failure" => Some(Self::Failure),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// Input to the audit writer (`ranch_sqlite::audit::record`). The writer
/// generates `id` and `created_at` inside the call so callers can't backdate
/// entries.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub source: AuditSource,
    pub action: AuditAction,
    pub target: AuditTarget,
    pub outcome: AuditOutcome,
    pub outcome_reason: Option<String>,
    /// Soft FK to `connections.id`. The connection may be deleted later
    /// Rows stay readable via the snapshot fields below.
    pub connection_id: Option<String>,
    /// Snapshot of the connection's display name at write time.
    pub connection_name: Option<String>,
    /// Snapshot of the connection's env-color token at write time.
    pub env_color: Option<String>,
    /// Populated when `source = AuditSource::Agent`.
    pub agent_client: Option<String>,
    pub agent_transport: Option<String>,
    /// The human approval that authorized this agent write, if
    /// one did (`mcp_pending_approvals.id`).
    pub approval_id: Option<String>,
}

/// Stored shape returned by `list_audit_log_entries` and emitted by
/// `export_audit_log`. Includes the auto-generated `id` and `created_at`.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditLogEntry {
    pub id: String,
    pub created_at: String,
    pub source: AuditSource,
    pub operator_id: String,
    pub agent_client: Option<String>,
    pub agent_transport: Option<String>,
    pub action: AuditAction,
    pub target: AuditTarget,
    pub outcome: AuditOutcome,
    pub outcome_reason: Option<String>,
    pub connection_id: Option<String>,
    pub connection_name: Option<String>,
    pub env_color: Option<String>,
    /// The group and environment the connection sits
    /// in *now*, joined at read time: three connections can share a name
    /// (one per environment) and the snapshot alone cannot tell them apart.
    /// `None` once the connection is deleted; the snapshot fields above stay.
    pub group_name: Option<String>,
    pub environment_name: Option<String>,
    /// Set when a human approval authorized this agent write.
    pub approval_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_and_outcome_string_mapping() {
        assert_eq!(AuditSource::Ui.as_str(), "ui");
        assert_eq!(AuditSource::try_from_str("ui"), Some(AuditSource::Ui));
        assert_eq!(AuditOutcome::Success.as_str(), "success");
        assert_eq!(
            AuditOutcome::try_from_str("failure"),
            Some(AuditOutcome::Failure)
        );
        assert_eq!(AuditOutcome::try_from_str("nope"), None);
    }
}
