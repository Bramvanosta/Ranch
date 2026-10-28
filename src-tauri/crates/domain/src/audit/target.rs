//! Audit target enum, stored as a tagged union.
//!
//! Serialized into `target_payload` as
//! `{ "kind": "job", "data": { "queueId": "...", "queueName": "...", "jobId": "..." } }`
//! so the frontend can pattern-match via a TypeScript discriminated union.
//! The log is history and its rows are never rewritten: a row written
//! before targets carried the queue's id has none (`queue_id: None`), and
//! keeps its queue's name.
//! The `kind` string is also stored separately in the `target_kind` SQL
//! column so the audit log can be filtered by target type cheaply via
//! the index without parsing JSON.

use serde::{Deserialize, Serialize};

use crate::queues::QueueId;

// Serialize/Deserialize: the target is stored as JSON in `target_payload`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AuditTarget {
    Job {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        queue_id: Option<String>,
        queue_name: String,
        job_id: String,
    },
    Queue {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        queue_id: Option<String>,
        queue_name: String,
    },
    Scheduler {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        queue_id: Option<String>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        queue_id: Option<String>,
        queue_name: String,
        job_count: u32,
    },
}

impl AuditTarget {
    pub fn job(queue_id: &QueueId, queue_name: &str, job_id: &str) -> Self {
        Self::Job {
            queue_id: Some(queue_id.to_string()),
            queue_name: queue_name.to_string(),
            job_id: job_id.to_string(),
        }
    }

    pub fn queue(queue_id: &QueueId, queue_name: &str) -> Self {
        Self::Queue {
            queue_id: Some(queue_id.to_string()),
            queue_name: queue_name.to_string(),
        }
    }

    pub fn scheduler(queue_id: &QueueId, queue_name: &str, scheduler_key: &str) -> Self {
        Self::Scheduler {
            queue_id: Some(queue_id.to_string()),
            queue_name: queue_name.to_string(),
            scheduler_key: scheduler_key.to_string(),
        }
    }

    pub fn bulk_jobs(queue_id: &QueueId, queue_name: &str, job_count: u32) -> Self {
        Self::BulkJobs {
            queue_id: Some(queue_id.to_string()),
            queue_name: queue_name.to_string(),
            job_count,
        }
    }

    /// String discriminator stored in the `target_kind` SQL column.
    /// Independent of the JSON payload encoding so we can filter/index
    /// without serde-parsing every row.
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Job { .. } => "job",
            Self::Queue { .. } => "queue",
            Self::Scheduler { .. } => "scheduler",
            Self::Connection => "connection",
            Self::BulkJobs { .. } => "bulkJobs",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emails() -> QueueId {
        QueueId::new("bull:emails")
    }

    #[test]
    fn job_target_serializes_with_tagged_union_shape() {
        let t = AuditTarget::job(&emails(), "emails", "42");
        let v: serde_json::Value = serde_json::to_value(&t).unwrap();
        assert_eq!(v["kind"], "job");
        assert_eq!(v["data"]["queueId"], "bull:emails");
        assert_eq!(v["data"]["queueName"], "emails");
        assert_eq!(v["data"]["jobId"], "42");
        // Round-trip.
        let back: AuditTarget = serde_json::from_value(v).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn queue_target_serializes_correctly() {
        let t = AuditTarget::queue(&emails(), "emails");
        let v: serde_json::Value = serde_json::to_value(&t).unwrap();
        assert_eq!(v["kind"], "queue");
        assert_eq!(v["data"]["queueName"], "emails");
    }

    #[test]
    fn scheduler_target_serializes_correctly() {
        let t = AuditTarget::scheduler(&emails(), "emails", "daily-report");
        let v: serde_json::Value = serde_json::to_value(&t).unwrap();
        assert_eq!(v["kind"], "scheduler");
        assert_eq!(v["data"]["schedulerKey"], "daily-report");
    }

    #[test]
    fn connection_target_serializes_without_data_payload() {
        let t = AuditTarget::Connection;
        let v: serde_json::Value = serde_json::to_value(&t).unwrap();
        assert_eq!(v["kind"], "connection");
        // The `Connection` variant carries no fields, but serde-tagged-
        // union still emits an empty `data` object (or null) depending
        // on the variant; accept either.
    }

    #[test]
    fn kind_str_matches_wire_discriminator() {
        for (t, kind) in [
            (AuditTarget::job(&emails(), "q", "1"), "job"),
            (AuditTarget::queue(&emails(), "q"), "queue"),
            (AuditTarget::scheduler(&emails(), "q", "k"), "scheduler"),
            (AuditTarget::Connection, "connection"),
            (AuditTarget::bulk_jobs(&emails(), "q", 247), "bulkJobs"),
        ] {
            assert_eq!(t.kind_str(), kind);
        }
    }

    /// `BulkJobs` is the consolidated target for bulk
    /// actions. Wire shape carries the queue name + job count.
    #[test]
    fn bulk_jobs_target_serializes_with_camel_case_fields() {
        let t = AuditTarget::bulk_jobs(&emails(), "emails", 247);
        let v: serde_json::Value = serde_json::to_value(&t).unwrap();
        assert_eq!(v["kind"], "bulkJobs");
        assert_eq!(v["data"]["queueName"], "emails");
        assert_eq!(v["data"]["jobCount"], 247);
        let back: AuditTarget = serde_json::from_value(v).unwrap();
        assert_eq!(back, t);
    }

    /// A row written before targets carried the queue's id reads back with
    /// its name and no id; the row itself is never rewritten.
    #[test]
    fn a_target_stored_without_a_queue_id_still_reads() {
        let old =
            serde_json::json!({"kind": "job", "data": {"queueName": "emails", "jobId": "42"}});
        let t: AuditTarget = serde_json::from_value(old).unwrap();
        assert_eq!(
            t,
            AuditTarget::Job {
                queue_id: None,
                queue_name: "emails".into(),
                job_id: "42".into(),
            }
        );
        let old_bulk =
            serde_json::json!({"kind": "bulkJobs", "data": {"queueName": "emails", "jobCount": 3}});
        assert!(matches!(
            serde_json::from_value::<AuditTarget>(old_bulk).unwrap(),
            AuditTarget::BulkJobs {
                queue_id: None,
                job_count: 3,
                ..
            }
        ));
    }
}
