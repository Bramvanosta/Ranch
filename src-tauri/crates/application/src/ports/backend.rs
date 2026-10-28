//! The queue backend: what Ranch reads and changes on one connection's
//! queues, whatever stores them.
//!
//! A backend is opened per connection by [`Backends`], which resolves the
//! connection's settings and secrets itself: operations only know an id.
//! It also lists the kinds of backend it serves ([`BackendSpec`]): what a
//! connection of each kind can do ([`Capabilities`]), what it is reached
//! at, how that endpoint is checked and how a server there is probed. The opened [`QueueBackend`]
//! declares its kind's capabilities and hands out families of methods. Reading queues and jobs is always
//! there; every other family is optional. An operation checks the
//! declaration before it calls, and answers [`RanchError::Unsupported`]
//! for what is not declared (see [`require`]).
//!
//! Arguments and results are the domain's types: the operations read the
//! contract's arguments into them and convert the answers for the wire. A
//! backend is opened for one connection, so no argument names it. Every
//! write takes the [`Caller`] and follows the write pipeline
//! (`application::writes`): authorize against the connection's lock or the
//! caller's approval, act, then audit before replying.

use std::sync::Arc;

use async_trait::async_trait;
use ranch_domain::audit::Caller;
use ranch_domain::capabilities::{Capabilities, Capability};
use ranch_domain::endpoint::{BackendKind, Endpoint, EndpointSchema};
use ranch_domain::errors::RanchError;
use ranch_domain::events::{EventsQuery, QueueEvents, QueueEventsNotice};
use ranch_domain::failures::{FailedJobsPage, FailedJobsQuery, FailureCauses};
use ranch_domain::flows::{FlowRoots, FlowTree};
use ranch_domain::jobs::{
    AddJobResult, BulkActionResult, Job, JobActionResult, JobBatch, JobLogs, JobLogsQuery, JobPage,
    JobPageQuery, JobTarget, NewJob, RecentJobs, RecentJobsQuery,
};
use ranch_domain::lock::ReadOnlyState;
use ranch_domain::maintenance::{MaintenancePreview, MaintenanceResult, QueueMaintenance};
use ranch_domain::metrics::{MetricsRange, QueueMetrics, QueueMetricsQuery, Throughput};
use ranch_domain::probe::{PingReport, ProbeReport, ProbeSecrets};
use ranch_domain::queues::{Queue, QueueActionResult, QueueId};
use ranch_domain::schedule::{
    CronJob, SchedulerActionResult, SchedulerSaved, SchedulerSpec, SchedulerTarget,
};
use ranch_domain::server::ServerInfo;
use ranch_domain::ssh_config::SshTunnelConfig;
use ranch_domain::timeline::{WindowQuery, WindowSummary};
use ranch_domain::topology::Topology;
use ranch_domain::workers::{QueueWorkerCounts, QueueWorkers};

use super::progress::ProgressSink;
use super::secrets::SecretStore;
use crate::ConnectionStamp;

/// Where a followed connection sends which queues moved: the app's
/// windows (`queue:events`), or a test.
pub type EventSink = Arc<dyn Fn(QueueEventsNotice) + Send + Sync>;

/// Opens the backend of a saved connection.
// `async_trait` marks the boxed future `must_use` on top of the `Result`.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait Backends: Send + Sync {
    /// The backend of `connection_id`, with its settings and secrets
    /// resolved. `ConnectionNotFound` when no such connection is saved.
    async fn open(&self, connection_id: &str) -> Result<Arc<dyn QueueBackend>, RanchError>;

    /// The kinds of backend a connection may name here.
    fn specs(&self) -> &BackendSpecs;
}

/// One kind of backend, as a connection names it: its label, what it can
/// do, what its endpoint holds, how an endpoint is checked and how the
/// server behind one is probed. Only the backend reads inside an endpoint;
/// the rest of Ranch asks it here.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait BackendSpec: Send + Sync {
    fn kind(&self) -> BackendKind;
    /// How the UI names the kind ("BullMQ on Redis").
    fn label(&self) -> &str;
    /// What every backend of the kind can do, and the job states it
    /// keeps: known without connecting, so the UI and the MCP tool list
    /// follow it. What a server refuses at run time (a Redis without
    /// `CLIENT LIST`) is an error or an empty state of the call, not part
    /// of it.
    fn capabilities(&self) -> &Capabilities;
    /// What the connection form asks for.
    fn endpoint_schema(&self) -> EndpointSchema;
    /// The endpoint as it is stored, or why it cannot be. Each field is
    /// checked; an empty optional one is left out.
    fn validate(&self, endpoint: &Endpoint) -> Result<Endpoint, RanchError>;
    /// Do two endpoints send the connection's secret to the same server?
    /// A saved secret is only ever sent back where it was saved for: when
    /// in doubt, they do not.
    fn same_server(&self, saved: &Endpoint, new: &Endpoint) -> bool;
    /// The SSH tunnel the endpoint goes through, if any. Its password is a
    /// secret of its own.
    fn tunnel(&self, endpoint: &Endpoint) -> Option<SshTunnelConfig>;
    /// Where the endpoint points, in one line and without credentials, as
    /// an agent reads it.
    fn describe(&self, endpoint: &Endpoint) -> String;
    /// Reach the server an endpoint (as [`validate`](Self::validate)
    /// stores it) points at with `secrets`, once, and report how it
    /// answered and what it holds. An unreachable server, a refused login
    /// or a failed tunnel is a report, not an error. An SSH tunnel's host
    /// key is pinned in `keychain` on first use.
    async fn probe(
        &self,
        endpoint: &Endpoint,
        secrets: &ProbeSecrets,
        keychain: Arc<dyn SecretStore>,
    ) -> Result<ProbeReport, RanchError>;
}

/// The kinds of backend a process serves, by kind.
#[derive(Clone, Default)]
pub struct BackendSpecs(Vec<Arc<dyn BackendSpec>>);

impl BackendSpecs {
    pub fn new(specs: Vec<Arc<dyn BackendSpec>>) -> Self {
        Self(specs)
    }

    /// The spec of `kind`, or `InvalidInput` when no backend here has it.
    pub fn get(&self, kind: &BackendKind) -> Result<&dyn BackendSpec, RanchError> {
        self.0
            .iter()
            .find(|s| &s.kind() == kind)
            .map(|s| s.as_ref())
            .ok_or_else(|| RanchError::InvalidInput {
                reason: format!("No backend of kind `{kind}` here"),
            })
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn BackendSpec> {
        self.0.iter().map(|s| s.as_ref())
    }
}

/// One connection's queues. The optional families default to absent, so
/// a backend declares only what it can do.
pub trait QueueBackend: Send + Sync {
    /// The connection as errors and audit rows name it.
    fn stamp(&self) -> &ConnectionStamp;

    /// What this backend can do, variant by variant: its kind's
    /// (`BackendSpec::capabilities`).
    fn capabilities(&self) -> &Capabilities;

    /// Reachability and the connection's lock.
    fn connection(&self) -> &dyn ConnectionControl;

    /// The queues and their counters.
    fn queues(&self) -> &dyn QueueReads;

    /// Jobs: lists, one job, its logs, what ran last, what failed.
    fn jobs(&self) -> &dyn JobReads;

    /// Retry, remove, promote (one job or many) and add jobs.
    fn job_actions(&self) -> Option<&dyn JobActions> {
        None
    }

    /// Whole-queue operations: drain, clean, retry every failed job.
    fn maintenance(&self) -> Option<&dyn Maintenance> {
        None
    }

    /// Pausing and resuming a queue.
    fn queue_control(&self) -> Option<&dyn QueueControl> {
        None
    }

    /// Job schedulers (cron patterns and fixed intervals).
    fn schedules(&self) -> Option<&dyn Schedules> {
        None
    }

    /// The workers consuming the queues.
    fn workers(&self) -> Option<&dyn Workers> {
        None
    }

    /// Parent and child jobs.
    fn flows(&self) -> Option<&dyn Flows> {
        None
    }

    /// Rates, durations and the timeline's summaries.
    fn metrics(&self) -> Option<&dyn Metrics> {
        None
    }

    /// The queues' live events.
    fn events(&self) -> Option<&dyn Events> {
        None
    }

    /// The server behind the connection and how its queues link up.
    fn server(&self) -> Option<&dyn ServerReads> {
        None
    }
}

/// The family an operation needs, when the backend declares `need` and
/// provides the family; `Unsupported` naming the capability otherwise.
pub fn require<'a, T: ?Sized>(
    capabilities: &Capabilities,
    need: Capability,
    family: Option<&'a T>,
) -> Result<&'a T, RanchError> {
    capabilities.ensure(need)?;
    family.ok_or_else(|| RanchError::Unsupported {
        capability: need.name().to_string(),
    })
}

// Each family below is an `async_trait`, which marks the boxed future
// `must_use` on top of the `Result`: hence the `double_must_use` allowance.

#[allow(clippy::double_must_use)]
#[async_trait]
pub trait ConnectionControl: Send + Sync {
    /// A reachability check. An unreachable server is a result, not an
    /// error: it says why.
    async fn ping(&self) -> Result<PingReport, RanchError>;
    /// The live lock state. An unlock lasts for the session.
    async fn lock_state(&self) -> Result<ReadOnlyState, RanchError>;
    async fn lock(&self) -> Result<ReadOnlyState, RanchError>;
    async fn unlock(&self) -> Result<ReadOnlyState, RanchError>;
}

#[allow(clippy::double_must_use)]
#[async_trait]
pub trait QueueReads: Send + Sync {
    /// Every queue of the connection, with its counters.
    async fn list_queues(&self) -> Result<Vec<Queue>, RanchError>;
}

#[allow(clippy::double_must_use)]
#[async_trait]
pub trait JobReads: Send + Sync {
    async fn list_jobs(&self, query: JobPageQuery) -> Result<JobPage, RanchError>;
    async fn get_job(&self, job: JobTarget) -> Result<Job, RanchError>;
    async fn get_job_logs(&self, query: JobLogsQuery) -> Result<JobLogs, RanchError>;
    async fn list_recent_jobs(&self, query: RecentJobsQuery) -> Result<RecentJobs, RanchError>;
    async fn list_failed_jobs(&self, query: FailedJobsQuery) -> Result<FailedJobsPage, RanchError>;
    /// The failed jobs of one queue (by name), or of every queue, grouped
    /// by cause.
    async fn list_failure_causes(&self, queue: Option<String>)
        -> Result<FailureCauses, RanchError>;
}

/// Writes on jobs. A bulk action is authorized once and audited as one
/// row, a refusal by the lock included; `progress` receives
/// `bulk:progress` events when a window is there to show them.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait JobActions: Send + Sync {
    async fn add_job(
        &self,
        queue_id: QueueId,
        job: NewJob,
        caller: Caller,
    ) -> Result<AddJobResult, RanchError>;
    async fn retry_job(
        &self,
        job: JobTarget,
        caller: Caller,
    ) -> Result<JobActionResult, RanchError>;
    async fn remove_job(
        &self,
        job: JobTarget,
        caller: Caller,
    ) -> Result<JobActionResult, RanchError>;
    async fn promote_job(
        &self,
        job: JobTarget,
        caller: Caller,
    ) -> Result<JobActionResult, RanchError>;
    async fn bulk_retry_jobs(
        &self,
        batch: JobBatch,
        progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<BulkActionResult, RanchError>;
    async fn bulk_remove_jobs(
        &self,
        batch: JobBatch,
        progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<BulkActionResult, RanchError>;
    async fn bulk_promote_jobs(
        &self,
        batch: JobBatch,
        progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<BulkActionResult, RanchError>;
}

#[allow(clippy::double_must_use)]
#[async_trait]
pub trait Maintenance: Send + Sync {
    /// How many jobs the operation would touch. Changes nothing.
    async fn preview(
        &self,
        queue_id: QueueId,
        operation: QueueMaintenance,
    ) -> Result<MaintenancePreview, RanchError>;
    async fn run(
        &self,
        queue_id: QueueId,
        operation: QueueMaintenance,
        progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<MaintenanceResult, RanchError>;
}

#[allow(clippy::double_must_use)]
#[async_trait]
pub trait QueueControl: Send + Sync {
    async fn pause_queue(
        &self,
        queue_id: QueueId,
        caller: Caller,
    ) -> Result<QueueActionResult, RanchError>;
    async fn resume_queue(
        &self,
        queue_id: QueueId,
        caller: Caller,
    ) -> Result<QueueActionResult, RanchError>;
}

#[allow(clippy::double_must_use)]
#[async_trait]
pub trait Schedules: Send + Sync {
    async fn list_schedulers(&self, queue_id: QueueId) -> Result<Vec<CronJob>, RanchError>;
    /// Saving an existing `spec.id` edits it.
    async fn upsert_scheduler(
        &self,
        queue_id: QueueId,
        spec: SchedulerSpec,
        caller: Caller,
    ) -> Result<SchedulerSaved, RanchError>;
    async fn remove_scheduler(
        &self,
        scheduler: SchedulerTarget,
        caller: Caller,
    ) -> Result<SchedulerActionResult, RanchError>;
    async fn pause_scheduler(
        &self,
        scheduler: SchedulerTarget,
        caller: Caller,
    ) -> Result<SchedulerActionResult, RanchError>;
    async fn resume_scheduler(
        &self,
        scheduler: SchedulerTarget,
        caller: Caller,
    ) -> Result<SchedulerActionResult, RanchError>;
}

#[allow(clippy::double_must_use)]
#[async_trait]
pub trait Workers: Send + Sync {
    async fn list_workers(&self, queue_id: QueueId) -> Result<QueueWorkers, RanchError>;
    async fn count_workers(&self, queue_ids: Vec<QueueId>)
        -> Result<QueueWorkerCounts, RanchError>;
}

#[allow(clippy::double_must_use)]
#[async_trait]
pub trait Flows: Send + Sync {
    /// The flow a job belongs to; a job in no flow is a tree of one.
    async fn get_flow(&self, job: JobTarget) -> Result<FlowTree, RanchError>;
    async fn list_flow_roots(&self, queue_id: QueueId) -> Result<FlowRoots, RanchError>;
}

#[allow(clippy::double_must_use)]
#[async_trait]
pub trait Metrics: Send + Sync {
    async fn queue_metrics(&self, query: QueueMetricsQuery) -> Result<QueueMetrics, RanchError>;
    /// Completed and failed over time for every queue of the connection.
    async fn throughput(&self, range: MetricsRange) -> Result<Throughput, RanchError>;
    async fn summarize_window(&self, window: WindowQuery) -> Result<WindowSummary, RanchError>;
}

#[allow(clippy::double_must_use)]
#[async_trait]
pub trait Events: Send + Sync {
    async fn read_events(&self, query: EventsQuery) -> Result<QueueEvents, RanchError>;
    /// Start or stop following; whether the connection is followed now.
    async fn follow(&self, follow: bool, sink: Option<EventSink>) -> Result<bool, RanchError>;
}

#[allow(clippy::double_must_use)]
#[async_trait]
pub trait ServerReads: Send + Sync {
    async fn server_info(&self) -> Result<ServerInfo, RanchError>;
    async fn topology(&self) -> Result<Topology, RanchError>;
}
