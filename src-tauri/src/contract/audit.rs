//! The audit log as the UI reads and exports it.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::queues::QueueId;
use crate::domain::audit;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AuditAction {
    RetryJob,
    RemoveJob,
    PromoteJob,
    PauseQueue,
    ResumeQueue,
    PauseScheduler,
    ResumeScheduler,
    UnlockConnection,
    LockConnection,
    /// A job enqueued from Ranch (UI or agent).
    AddJob,
    /// queue-level maintenance.
    DrainQueue,
    CleanQueue,
    RetryAllFailed,
    /// A job scheduler created / changed, or deleted.
    UpsertScheduler,
    RemoveScheduler,
}

impl From<audit::AuditAction> for AuditAction {
    fn from(v: audit::AuditAction) -> Self {
        match v {
            audit::AuditAction::RetryJob => Self::RetryJob,
            audit::AuditAction::RemoveJob => Self::RemoveJob,
            audit::AuditAction::PromoteJob => Self::PromoteJob,
            audit::AuditAction::PauseQueue => Self::PauseQueue,
            audit::AuditAction::ResumeQueue => Self::ResumeQueue,
            audit::AuditAction::PauseScheduler => Self::PauseScheduler,
            audit::AuditAction::ResumeScheduler => Self::ResumeScheduler,
            audit::AuditAction::UnlockConnection => Self::UnlockConnection,
            audit::AuditAction::LockConnection => Self::LockConnection,
            audit::AuditAction::AddJob => Self::AddJob,
            audit::AuditAction::DrainQueue => Self::DrainQueue,
            audit::AuditAction::CleanQueue => Self::CleanQueue,
            audit::AuditAction::RetryAllFailed => Self::RetryAllFailed,
            audit::AuditAction::UpsertScheduler => Self::UpsertScheduler,
            audit::AuditAction::RemoveScheduler => Self::RemoveScheduler,
        }
    }
}

/// Who initiated the action: a human via the Ranch UI, or an agent
/// via the MCP server (the MCP server populates the agent path).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AuditSource {
    Ui,
    Agent,
}

impl From<audit::AuditSource> for AuditSource {
    fn from(v: audit::AuditSource) -> Self {
        match v {
            audit::AuditSource::Ui => Self::Ui,
            audit::AuditSource::Agent => Self::Agent,
        }
    }
}

/// Outcome of the action: did it succeed, fail, or cancel mid-flight?
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AuditOutcome {
    Success,
    Failure,
    Cancelled,
}

impl From<audit::AuditOutcome> for AuditOutcome {
    fn from(v: audit::AuditOutcome) -> Self {
        match v {
            audit::AuditOutcome::Success => Self::Success,
            audit::AuditOutcome::Failure => Self::Failure,
            audit::AuditOutcome::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AuditTarget {
    /// A row written before targets carried the queue's id has none.
    Job {
        queue_id: Option<QueueId>,
        queue_name: String,
        job_id: String,
    },
    Queue {
        queue_id: Option<QueueId>,
        queue_name: String,
    },
    Scheduler {
        queue_id: Option<QueueId>,
        queue_name: String,
        scheduler_key: String,
    },
    Connection,
    /// A bulk action over a list of jobs within a single
    /// queue. ONE audit row per bulk attempt (regardless of how many
    /// jobs were touched). The `outcome_reason` field carries a
    /// summary of partial-failure detail. The frontend's
    /// `formatAuditTarget` renders this as `{queueName} / {jobCount} jobs`.
    BulkJobs {
        queue_id: Option<QueueId>,
        queue_name: String,
        job_count: u32,
    },
}

impl From<audit::AuditTarget> for AuditTarget {
    fn from(v: audit::AuditTarget) -> Self {
        match v {
            audit::AuditTarget::Job {
                queue_id,
                queue_name,
                job_id,
            } => Self::Job {
                queue_id: queue_id.map(QueueId),
                queue_name,
                job_id,
            },
            audit::AuditTarget::Queue {
                queue_id,
                queue_name,
            } => Self::Queue {
                queue_id: queue_id.map(QueueId),
                queue_name,
            },
            audit::AuditTarget::Scheduler {
                queue_id,
                queue_name,
                scheduler_key,
            } => Self::Scheduler {
                queue_id: queue_id.map(QueueId),
                queue_name,
                scheduler_key,
            },
            audit::AuditTarget::Connection => Self::Connection,
            audit::AuditTarget::BulkJobs {
                queue_id,
                queue_name,
                job_count,
            } => Self::BulkJobs {
                queue_id: queue_id.map(QueueId),
                queue_name,
                job_count,
            },
        }
    }
}

/// Stored shape returned by `list_audit_log_entries` and emitted by
/// `export_audit_log`. Includes the auto-generated `id` and `created_at`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
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

impl From<audit::AuditLogEntry> for AuditLogEntry {
    fn from(v: audit::AuditLogEntry) -> Self {
        Self {
            id: v.id,
            created_at: v.created_at,
            source: v.source.into(),
            operator_id: v.operator_id,
            agent_client: v.agent_client,
            agent_transport: v.agent_transport,
            action: v.action.into(),
            target: v.target.into(),
            outcome: v.outcome.into(),
            outcome_reason: v.outcome_reason,
            connection_id: v.connection_id,
            connection_name: v.connection_name,
            env_color: v.env_color,
            group_name: v.group_name,
            environment_name: v.environment_name,
            approval_id: v.approval_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The wire names are the strings the audit log stores.
    #[test]
    fn actions_cross_the_wire_as_they_are_stored() {
        for v in [
            audit::AuditAction::RetryJob,
            audit::AuditAction::RemoveJob,
            audit::AuditAction::PromoteJob,
            audit::AuditAction::PauseQueue,
            audit::AuditAction::ResumeQueue,
            audit::AuditAction::PauseScheduler,
            audit::AuditAction::ResumeScheduler,
            audit::AuditAction::UnlockConnection,
            audit::AuditAction::LockConnection,
            audit::AuditAction::AddJob,
            audit::AuditAction::DrainQueue,
            audit::AuditAction::CleanQueue,
            audit::AuditAction::RetryAllFailed,
            audit::AuditAction::UpsertScheduler,
            audit::AuditAction::RemoveScheduler,
        ] {
            let json = serde_json::to_string(&AuditAction::from(v)).unwrap();
            assert_eq!(json.trim_matches('"'), v.as_str(), "mismatch for {v:?}");
        }
        for v in [audit::AuditSource::Ui, audit::AuditSource::Agent] {
            let json = serde_json::to_string(&AuditSource::from(v)).unwrap();
            assert_eq!(json.trim_matches('"'), v.as_str());
        }
        for v in [
            audit::AuditOutcome::Success,
            audit::AuditOutcome::Failure,
            audit::AuditOutcome::Cancelled,
        ] {
            let json = serde_json::to_string(&AuditOutcome::from(v)).unwrap();
            assert_eq!(json.trim_matches('"'), v.as_str());
        }
    }

    #[test]
    fn audit_log_entry_round_trips_through_camel_case() {
        let entry = AuditLogEntry::from(audit::AuditLogEntry {
            id: "0193fef0-1234-7000-8000-deadbeefcafe".into(),
            created_at: "2026-05-11T12:00:00Z".into(),
            source: audit::AuditSource::Ui,
            operator_id: "local".into(),
            agent_client: None,
            agent_transport: None,
            approval_id: None,
            action: audit::AuditAction::RetryJob,
            target: audit::AuditTarget::job(
                &crate::domain::queues::QueueId::new("bull:emails"),
                "emails",
                "42",
            ),
            outcome: audit::AuditOutcome::Success,
            outcome_reason: None,
            connection_id: Some("conn-x".into()),
            connection_name: Some("Acme · prod".into()),
            env_color: Some("env-production".into()),
            group_name: Some("Acme".into()),
            environment_name: Some("Production".into()),
        });
        let v: serde_json::Value = serde_json::to_value(&entry).unwrap();
        assert_eq!(v["createdAt"], "2026-05-11T12:00:00Z");
        assert_eq!(v["operatorId"], "local");
        assert_eq!(v["source"], "ui");
        assert_eq!(v["action"], "retryJob");
        assert_eq!(v["target"]["kind"], "job");
        assert_eq!(v["target"]["data"]["queueName"], "emails");
        assert_eq!(v["target"]["data"]["queueId"], "bull:emails");
        assert_eq!(v["envColor"], "env-production");
        assert_eq!(v["groupName"], "Acme");
        assert_eq!(v["environmentName"], "Production");
        let back: AuditLogEntry = serde_json::from_value(v).unwrap();
        assert_eq!(back, entry);
    }
}
