//! Audit target enum (tagged union on the wire).
//!
//! Serialized as `{ "kind": "job", "data": { "queueName": "...", "jobId": "..." } }`
//! so the frontend can pattern-match via a TypeScript discriminated union.
//! The `kind` string is also stored separately in the `target_kind` SQL
//! column so the audit log can be filtered by target type cheaply via
//! the index without parsing JSON.

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AuditTarget {
    Job {
        queue_name: String,
        job_id: String,
    },
    Queue {
        queue_name: String,
    },
    Scheduler {
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
        queue_name: String,
        job_count: u32,
    },
}

impl AuditTarget {
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

    #[test]
    fn job_target_serializes_with_tagged_union_shape() {
        let t = AuditTarget::Job {
            queue_name: "emails".into(),
            job_id: "42".into(),
        };
        let v: serde_json::Value = serde_json::to_value(&t).unwrap();
        assert_eq!(v["kind"], "job");
        assert_eq!(v["data"]["queueName"], "emails");
        assert_eq!(v["data"]["jobId"], "42");
        // Round-trip.
        let back: AuditTarget = serde_json::from_value(v).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn queue_target_serializes_correctly() {
        let t = AuditTarget::Queue {
            queue_name: "emails".into(),
        };
        let v: serde_json::Value = serde_json::to_value(&t).unwrap();
        assert_eq!(v["kind"], "queue");
        assert_eq!(v["data"]["queueName"], "emails");
    }

    #[test]
    fn scheduler_target_serializes_correctly() {
        let t = AuditTarget::Scheduler {
            queue_name: "emails".into(),
            scheduler_key: "daily-report".into(),
        };
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
        // on the variant — accept either.
    }

    #[test]
    fn kind_str_matches_wire_discriminator() {
        for (t, kind) in [
            (
                AuditTarget::Job {
                    queue_name: "q".into(),
                    job_id: "1".into(),
                },
                "job",
            ),
            (
                AuditTarget::Queue {
                    queue_name: "q".into(),
                },
                "queue",
            ),
            (
                AuditTarget::Scheduler {
                    queue_name: "q".into(),
                    scheduler_key: "k".into(),
                },
                "scheduler",
            ),
            (AuditTarget::Connection, "connection"),
            (
                AuditTarget::BulkJobs {
                    queue_name: "q".into(),
                    job_count: 247,
                },
                "bulkJobs",
            ),
        ] {
            assert_eq!(t.kind_str(), kind);
        }
    }

    /// `BulkJobs` is the consolidated target for bulk
    /// actions. Wire shape carries the queue name + job count.
    #[test]
    fn bulk_jobs_target_serializes_with_camel_case_fields() {
        let t = AuditTarget::BulkJobs {
            queue_name: "emails".into(),
            job_count: 247,
        };
        let v: serde_json::Value = serde_json::to_value(&t).unwrap();
        assert_eq!(v["kind"], "bulkJobs");
        assert_eq!(v["data"]["queueName"], "emails");
        assert_eq!(v["data"]["jobCount"], 247);
        let back: AuditTarget = serde_json::from_value(v).unwrap();
        assert_eq!(back, t);
    }
}
