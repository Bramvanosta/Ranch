//! BullMQ schema decoder module.
//!
//! THE ONLY PLACE in Ranch that knows BullMQ's Redis-key conventions and
//! field encoding. Feature code consumes the typed boundary types
//! ([`Queue`], [`Job`], etc.) via this module's public API and the
//! [`Keys`] / [`JobDecoder`] traits — never by reaching into raw Redis
//! keys, never by importing `crate::core::bullmq_schema::v5::*` directly.
//!
//! ## Adding a new BullMQ minor
//!
//!   1. Drop fixtures in `fixtures/vN/`.
//!   2. Add a `vN.rs` decoder branch if the field encoding drifts.
//!   3. Extend [`detect_schema`] to recognize the version.
//!
//! No feature code changes; the CI test (`tests/bullmq_schema_versions.rs`)
//! catches regressions on every commit.
//!
//! ## Architectural rule (non-negotiable)
//!
//! Grep for `format!("bull:` or `"bull:` string literals OUTSIDE
//! `core::bullmq_schema::v5` and `core::bullmq_schema::v6` should return
//! ZERO hits. Adding a new feature that builds a Redis key for BullMQ?
//! Use the [`Keys`] trait. Never bypass it.

pub mod add_job;
mod cron;
mod decode_helpers;
mod domain;
pub mod events;
pub mod failed;
pub mod flow;
pub mod job_query;
mod keys;
pub mod maintenance;
pub mod metrics;
pub mod recent;
pub mod scheduler;
mod schema;
pub mod timeline;
pub mod topology;
pub mod v5;
pub mod v6;
pub mod workers;

pub use add_job::NewJob;
pub use cron::decode_cron_hash;
pub use domain::{CronJob, Job, JobRef, JobState, Queue, QueueCounters, QueueRef, StateRedisShape};
pub use failed::{FailedJobsPage, FailedQueueCount, FailureCause, FailureCauses};
pub use flow::{FlowNode, FlowRoot, FlowRoots, FlowTree};
pub use job_query::{JobSort, JobSortField};
pub use keys::{KeyVariant, Keys};
pub use maintenance::{CleanState, MaintenancePreview, MaintenanceResult, QueueMaintenance};
pub use metrics::{
    FailingJobType, MetricsRange, QueueMetrics, QueueThroughput, SlowJob, Throughput, UsualDuration,
};
pub use recent::RecentJobs;
pub use scheduler::{Schedule, SchedulerSaved, SchedulerSpec};
pub use schema::{detect_schema, BullmqSchema, JobDecoder, SchemaError};
pub use workers::{QueueWorker, QueueWorkerCount, QueueWorkerCounts, QueueWorkers};

// Re-export the per-version Decoder + Keys types so callers (the CI fixture
// test, the ConnectionActor) can hold concrete instances without
// reaching into the `v5` / `v6` submodule internals. The submodules
// themselves remain isolated — feature code uses these re-exports OR the
// `KeyVariant` dispatch enum, NEVER direct paths like `v5::SomeInternalFn`.
pub use v5::{V5Decoder, V5Keys};
pub use v6::{V6Decoder, V6Keys, MARKER_TOKEN};
