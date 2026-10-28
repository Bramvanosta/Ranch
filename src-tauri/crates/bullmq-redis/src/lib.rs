//! Queue backends for BullMQ on Redis.
//!
//! Each connection is served by its connection actor ([`connection_actor`]):
//! one persistent Redis link, reads run concurrently, writes one at a time
//! with the write pipeline around each. This adapter resolves a connection's
//! settings from the saved connections and the keychain
//! ([`resolve_connection_config`]), and forwards every operation to the actor
//! through the registry, which spawns it on first use. The connection's
//! endpoint is read in [`endpoint`], the only place that knows its shape; a
//! Redis is probed before its connection is saved in [`probe`]. What BullMQ
//! keeps in Redis is read in [`bullmq_schema`]; [`ssh_tunnel`] and [`tls`]
//! reach the server.
//!
//! A crate of its own: it reaches the rest of Ranch through the
//! application's ports alone (the composition roots hand it the write
//! pipeline, the saved connections and the secret store), and its manifest
//! names no other store, shell or wire format. The app crate re-exports it as
//! `adapters::bullmq_redis`.

// The backends answer `RanchError`; see the domain crate's note on
// `result_large_err`.
#![allow(clippy::result_large_err)]

pub mod bullmq_schema;
pub mod connection_actor;
pub mod endpoint;
pub mod probe;
pub mod redis_info;
pub mod ssh_tunnel;
pub mod tls;

use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use ranch_application::ports::backend::{
    BackendSpecs, Backends, ConnectionControl, EventSink, Events, Flows, JobActions, JobReads,
    Maintenance, Metrics, QueueBackend, QueueControl, QueueReads, Schedules, ServerReads, Workers,
};
use ranch_application::ports::connections::ConnectionRepository;
use ranch_application::ports::secrets::password_entry;
use ranch_application::ports::ProgressSink;
use ranch_application::ports::SecretStore;
use ranch_application::{ConnectionStamp, WritePipeline};
use ranch_domain::audit::Caller;
use ranch_domain::capabilities::{
    Capabilities, EventSupport, JobActionSupport, MaintenanceSupport, MetricsSupport,
    ScheduleSupport, ServerSupport, WorkerSupport,
};
use ranch_domain::errors::RanchError;
use ranch_domain::events::{EventsQuery, QueueEvents};
use ranch_domain::failures::{FailedJobsPage, FailedJobsQuery, FailureCauses};
use ranch_domain::flows::{FlowRoots, FlowTree};
use ranch_domain::jobs::{
    AddJobResult, BulkActionResult, Job, JobActionResult, JobBatch, JobLogs, JobLogsQuery, JobPage,
    JobPageQuery, JobTarget, NewJob, RecentJobs, RecentJobsQuery,
};
use ranch_domain::lock::ReadOnlyState;
use ranch_domain::maintenance::{MaintenancePreview, MaintenanceResult, QueueMaintenance};
use ranch_domain::metrics::{MetricsRange, QueueMetrics, QueueMetricsQuery, Throughput};
use ranch_domain::probe::PingReport;
use ranch_domain::queues::{Queue, QueueActionResult, QueueId};
use ranch_domain::schedule::{
    CronJob, SchedulerActionResult, SchedulerSaved, SchedulerSpec, SchedulerTarget,
};
use ranch_domain::server::ServerInfo;
use ranch_domain::ssh_config::SshAuthMethod;
use ranch_domain::timeline::{WindowQuery, WindowSummary};
use ranch_domain::topology::Topology;
use ranch_domain::workers::{QueueWorkerCounts, QueueWorkers};

use crate::bullmq_schema::states;
use crate::connection_actor::{ConnectionConfig, ConnectionRegistry};
use crate::endpoint::{RedisEndpoint, BULLMQ_REDIS};

/// Opens BullMQ-on-Redis backends for the connections saved in `ranch.db`.
pub struct BullmqRedisBackends {
    /// What each actor runs every write through: where it records outcomes
    /// and trades approvals.
    writes: Arc<WritePipeline>,
    specs: BackendSpecs,
    connections: Arc<dyn ConnectionRepository>,
    keychain: Arc<dyn SecretStore>,
    registry: ConnectionRegistry,
}

impl BullmqRedisBackends {
    /// `registry` is shared with whoever shuts actors down when a
    /// connection is edited or removed.
    pub fn new(
        writes: Arc<WritePipeline>,
        connections: Arc<dyn ConnectionRepository>,
        keychain: Arc<dyn SecretStore>,
        registry: ConnectionRegistry,
    ) -> Self {
        Self {
            writes,
            specs: specs(),
            connections,
            keychain,
            registry,
        }
    }
}

#[async_trait]
impl Backends for BullmqRedisBackends {
    async fn open(&self, connection_id: &str) -> Result<Arc<dyn QueueBackend>, RanchError> {
        let config = resolve_connection_config(
            connection_id,
            self.connections.as_ref(),
            &self.keychain,
            Arc::clone(&self.writes),
        )
        .await?;
        Ok(Arc::new(BullmqRedisBackend {
            id: connection_id.to_string(),
            stamp: config.stamp(),
            config,
            registry: self.registry.clone(),
        }))
    }

    fn specs(&self) -> &BackendSpecs {
        &self.specs
    }
}

/// A saved connection's actor settings, from its stored row and its
/// keychain entries. The BullMQ backend opens connections with it; `writes`
/// is the pipeline its actor runs every write through.
///
/// A row whose seal does not hold is refused before any secret is read: in
/// `ranch-mcp` (the one process that checks seals) the endpoint, the lock
/// and the password's name all come from rows the app sealed.
pub async fn resolve_connection_config(
    connection_id: &str,
    connections: &dyn ConnectionRepository,
    keychain: &Arc<dyn SecretStore>,
    writes: Arc<WritePipeline>,
) -> Result<ConnectionConfig, RanchError> {
    let stored = connections
        .stored_settings(connection_id)
        .await?
        .ok_or_else(|| RanchError::ConnectionNotFound {
            id: connection_id.to_string(),
        })?;

    if !stored.sealed {
        return Err(RanchError::ConnectionUnverified {
            connection_name: stored.name,
        });
    }
    if stored.backend.as_str() != BULLMQ_REDIS {
        return Err(RanchError::InvalidInput {
            reason: format!("No backend of kind `{}` here", stored.backend),
        });
    }
    let endpoint = RedisEndpoint::parse(&stored.endpoint)?;

    let password = keychain.get(password_entry(connection_id)?)?;

    // Resolve the SSH password too when the tunnel uses
    // password auth. Stored at `{id}::ssh`.
    let ssh_password = match endpoint.ssh_tunnel.as_ref().map(|c| &c.auth) {
        Some(SshAuthMethod::Password) => {
            keychain.get(&ssh_tunnel::ssh_keychain_name(connection_id))?
        }
        _ => None,
    };

    let initial_lock_state = if stored.locked {
        ReadOnlyState::Locked
    } else {
        ReadOnlyState::Unlocked
    };

    Ok(ConnectionConfig {
        endpoint,
        password,
        initial_lock_state,
        connection_id: connection_id.to_string(),
        connection_name: stored.name,
        env_color: stored.env_color,
        writes,
        // The keychain Arc serves the actor's hostkey-pin checks.
        ssh_password,
        keychain: Arc::clone(keychain),
    })
}

/// The kinds of backend this adapter serves: BullMQ on Redis.
pub fn specs() -> BackendSpecs {
    BackendSpecs::new(vec![Arc::new(endpoint::BullmqRedisSpec)])
}

/// One connection. Every call goes to its actor; the outer error of a
/// reply (the actor is gone or never answered) becomes `BackendUnavailable`.
struct BullmqRedisBackend {
    id: String,
    stamp: ConnectionStamp,
    /// Used only when the actor is not running yet: a running actor keeps
    /// the settings it was spawned with.
    config: ConnectionConfig,
    registry: ConnectionRegistry,
}

/// BullMQ on Redis does everything Ranch knows, in every state BullMQ keeps.
/// The kind's spec declares it and every connection's backend answers it.
static CAPABILITIES: LazyLock<Capabilities> = LazyLock::new(|| Capabilities {
    job_states: states::LISTED.to_vec(),
    job_actions: JobActionSupport {
        single: true,
        bulk: true,
        add: true,
    },
    maintenance: MaintenanceSupport {
        drain: true,
        clean: true,
        retry_all_failed: true,
    },
    queue_control: true,
    schedules: ScheduleSupport {
        list: true,
        write: true,
        pause: true,
    },
    workers: WorkerSupport {
        list: true,
        count: true,
    },
    flows: true,
    metrics: MetricsSupport {
        queue: true,
        throughput: true,
        timeline: true,
        native: true,
    },
    events: EventSupport {
        read: true,
        follow: true,
    },
    server: ServerSupport {
        info: true,
        topology: true,
    },
});

impl QueueBackend for BullmqRedisBackend {
    fn stamp(&self) -> &ConnectionStamp {
        &self.stamp
    }
    fn capabilities(&self) -> &Capabilities {
        &CAPABILITIES
    }
    fn connection(&self) -> &dyn ConnectionControl {
        self
    }
    fn queues(&self) -> &dyn QueueReads {
        self
    }
    fn jobs(&self) -> &dyn JobReads {
        self
    }
    fn job_actions(&self) -> Option<&dyn JobActions> {
        Some(self)
    }
    fn maintenance(&self) -> Option<&dyn Maintenance> {
        Some(self)
    }
    fn queue_control(&self) -> Option<&dyn QueueControl> {
        Some(self)
    }
    fn schedules(&self) -> Option<&dyn Schedules> {
        Some(self)
    }
    fn workers(&self) -> Option<&dyn Workers> {
        Some(self)
    }
    fn flows(&self) -> Option<&dyn Flows> {
        Some(self)
    }
    fn metrics(&self) -> Option<&dyn Metrics> {
        Some(self)
    }
    fn events(&self) -> Option<&dyn Events> {
        Some(self)
    }
    fn server(&self) -> Option<&dyn ServerReads> {
        Some(self)
    }
}

#[async_trait]
impl ConnectionControl for BullmqRedisBackend {
    async fn ping(&self) -> Result<PingReport, RanchError> {
        Ok(self.registry.ping(&self.id, &self.config).await?)
    }
    async fn lock_state(&self) -> Result<ReadOnlyState, RanchError> {
        Ok(self.registry.get_lock_state(&self.id, &self.config).await?)
    }
    async fn lock(&self) -> Result<ReadOnlyState, RanchError> {
        self.registry.lock(&self.id, &self.config).await?
    }
    async fn unlock(&self) -> Result<ReadOnlyState, RanchError> {
        self.registry.unlock(&self.id, &self.config).await?
    }
}

#[async_trait]
impl QueueReads for BullmqRedisBackend {
    async fn list_queues(&self) -> Result<Vec<Queue>, RanchError> {
        self.registry.list_queues(&self.id, &self.config).await?
    }
}

#[async_trait]
impl JobReads for BullmqRedisBackend {
    async fn list_jobs(&self, query: JobPageQuery) -> Result<JobPage, RanchError> {
        self.registry
            .list_jobs(&self.id, &self.config, query)
            .await?
    }
    async fn get_job(&self, job: JobTarget) -> Result<Job, RanchError> {
        self.registry.get_job(&self.id, &self.config, job).await?
    }
    async fn get_job_logs(&self, query: JobLogsQuery) -> Result<JobLogs, RanchError> {
        self.registry
            .get_job_logs(&self.id, &self.config, query)
            .await?
    }
    async fn list_recent_jobs(&self, query: RecentJobsQuery) -> Result<RecentJobs, RanchError> {
        self.registry
            .list_recent_jobs(&self.id, &self.config, query)
            .await?
    }
    async fn list_failed_jobs(&self, query: FailedJobsQuery) -> Result<FailedJobsPage, RanchError> {
        self.registry
            .list_failed_jobs(&self.id, &self.config, query)
            .await?
    }
    async fn list_failure_causes(
        &self,
        queue: Option<String>,
    ) -> Result<FailureCauses, RanchError> {
        self.registry
            .list_failure_causes(&self.id, &self.config, queue)
            .await?
    }
}

#[async_trait]
impl JobActions for BullmqRedisBackend {
    async fn add_job(
        &self,
        queue_id: QueueId,
        job: NewJob,
        caller: Caller,
    ) -> Result<AddJobResult, RanchError> {
        self.registry
            .add_job(&self.id, &self.config, queue_id, job, caller)
            .await?
    }
    async fn retry_job(
        &self,
        job: JobTarget,
        caller: Caller,
    ) -> Result<JobActionResult, RanchError> {
        self.registry
            .retry_job(&self.id, &self.config, job, caller)
            .await?
    }
    async fn remove_job(
        &self,
        job: JobTarget,
        caller: Caller,
    ) -> Result<JobActionResult, RanchError> {
        self.registry
            .remove_job(&self.id, &self.config, job, caller)
            .await?
    }
    async fn promote_job(
        &self,
        job: JobTarget,
        caller: Caller,
    ) -> Result<JobActionResult, RanchError> {
        self.registry
            .promote_job(&self.id, &self.config, job, caller)
            .await?
    }
    async fn bulk_retry_jobs(
        &self,
        batch: JobBatch,
        progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<BulkActionResult, RanchError> {
        self.registry
            .bulk_retry_jobs(&self.id, &self.config, batch, progress, caller)
            .await?
    }
    async fn bulk_remove_jobs(
        &self,
        batch: JobBatch,
        progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<BulkActionResult, RanchError> {
        self.registry
            .bulk_remove_jobs(&self.id, &self.config, batch, progress, caller)
            .await?
    }
    async fn bulk_promote_jobs(
        &self,
        batch: JobBatch,
        progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<BulkActionResult, RanchError> {
        self.registry
            .bulk_promote_jobs(&self.id, &self.config, batch, progress, caller)
            .await?
    }
}

#[async_trait]
impl Maintenance for BullmqRedisBackend {
    async fn preview(
        &self,
        queue_id: QueueId,
        operation: QueueMaintenance,
    ) -> Result<MaintenancePreview, RanchError> {
        self.registry
            .preview_queue_maintenance(&self.id, &self.config, queue_id, operation)
            .await?
    }
    async fn run(
        &self,
        queue_id: QueueId,
        operation: QueueMaintenance,
        progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<MaintenanceResult, RanchError> {
        self.registry
            .run_queue_maintenance(
                &self.id,
                &self.config,
                queue_id,
                operation,
                progress,
                caller,
            )
            .await?
    }
}

#[async_trait]
impl QueueControl for BullmqRedisBackend {
    async fn pause_queue(
        &self,
        queue_id: QueueId,
        caller: Caller,
    ) -> Result<QueueActionResult, RanchError> {
        self.registry
            .pause_queue(&self.id, &self.config, queue_id, caller)
            .await?
    }
    async fn resume_queue(
        &self,
        queue_id: QueueId,
        caller: Caller,
    ) -> Result<QueueActionResult, RanchError> {
        self.registry
            .resume_queue(&self.id, &self.config, queue_id, caller)
            .await?
    }
}

#[async_trait]
impl Schedules for BullmqRedisBackend {
    async fn list_schedulers(&self, queue_id: QueueId) -> Result<Vec<CronJob>, RanchError> {
        self.registry
            .list_cron_jobs(&self.id, &self.config, queue_id)
            .await?
    }
    async fn upsert_scheduler(
        &self,
        queue_id: QueueId,
        spec: SchedulerSpec,
        caller: Caller,
    ) -> Result<SchedulerSaved, RanchError> {
        self.registry
            .upsert_scheduler(&self.id, &self.config, queue_id, spec, caller)
            .await?
    }
    async fn remove_scheduler(
        &self,
        scheduler: SchedulerTarget,
        caller: Caller,
    ) -> Result<SchedulerActionResult, RanchError> {
        self.registry
            .remove_scheduler(&self.id, &self.config, scheduler, caller)
            .await?
    }
    async fn pause_scheduler(
        &self,
        scheduler: SchedulerTarget,
        caller: Caller,
    ) -> Result<SchedulerActionResult, RanchError> {
        self.registry
            .pause_scheduler(&self.id, &self.config, scheduler, caller)
            .await?
    }
    async fn resume_scheduler(
        &self,
        scheduler: SchedulerTarget,
        caller: Caller,
    ) -> Result<SchedulerActionResult, RanchError> {
        self.registry
            .resume_scheduler(&self.id, &self.config, scheduler, caller)
            .await?
    }
}

#[async_trait]
impl Workers for BullmqRedisBackend {
    async fn list_workers(&self, queue_id: QueueId) -> Result<QueueWorkers, RanchError> {
        self.registry
            .list_workers(&self.id, &self.config, queue_id)
            .await?
    }
    async fn count_workers(
        &self,
        queue_ids: Vec<QueueId>,
    ) -> Result<QueueWorkerCounts, RanchError> {
        self.registry
            .count_queue_workers(&self.id, &self.config, queue_ids)
            .await?
    }
}

#[async_trait]
impl Flows for BullmqRedisBackend {
    async fn get_flow(&self, job: JobTarget) -> Result<FlowTree, RanchError> {
        self.registry.get_flow(&self.id, &self.config, job).await?
    }
    async fn list_flow_roots(&self, queue_id: QueueId) -> Result<FlowRoots, RanchError> {
        self.registry
            .list_flow_roots(&self.id, &self.config, queue_id)
            .await?
    }
}

#[async_trait]
impl Metrics for BullmqRedisBackend {
    async fn queue_metrics(&self, query: QueueMetricsQuery) -> Result<QueueMetrics, RanchError> {
        self.registry
            .get_queue_metrics(&self.id, &self.config, query)
            .await?
    }
    async fn throughput(&self, range: MetricsRange) -> Result<Throughput, RanchError> {
        self.registry
            .get_throughput(&self.id, &self.config, range)
            .await?
    }
    async fn summarize_window(&self, window: WindowQuery) -> Result<WindowSummary, RanchError> {
        self.registry
            .summarize_window(&self.id, &self.config, window)
            .await?
    }
}

#[async_trait]
impl Events for BullmqRedisBackend {
    async fn read_events(&self, query: EventsQuery) -> Result<QueueEvents, RanchError> {
        self.registry
            .read_queue_events(&self.id, &self.config, query)
            .await?
    }
    async fn follow(&self, follow: bool, sink: Option<EventSink>) -> Result<bool, RanchError> {
        Ok(self
            .registry
            .follow_events(&self.id, &self.config, follow, sink)
            .await?)
    }
}

#[async_trait]
impl ServerReads for BullmqRedisBackend {
    async fn server_info(&self) -> Result<ServerInfo, RanchError> {
        self.registry
            .get_server_info(&self.id, &self.config)
            .await?
    }
    async fn topology(&self) -> Result<Topology, RanchError> {
        self.registry.get_topology(&self.id, &self.config).await?
    }
}
