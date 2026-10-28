//! caller-side handle to a running [`ConnectionActor`].
//!
//! Cheap to clone (just an `mpsc::Sender` clone + a `CancellationToken`
//! clone). [`ConnectionRegistry`](super::registry::ConnectionRegistry)
//! keeps one per active connection_id.
//!
//! Note: the registry holds the actor's `JoinHandle`, not the handle.
//! Callers don't need lifecycle control; they just send messages.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::actor::ActorCommand;
use crate::bullmq_schema::job_query::JobQuery;
use crate::bullmq_schema::{
    CronJob, FlowRoots, FlowTree, Job, MaintenancePreview, MaintenanceResult, QueueWorkerCounts,
    QueueWorkers, RecentJobs, SchedulerSaved,
};
use ranch_application::ports::ProgressSink;
use ranch_domain::audit::Caller;
use ranch_domain::errors::RanchError;
use ranch_domain::events::{EventsQuery, QueueEvents};
use ranch_domain::failures::{FailedJobsPage, FailedJobsQuery, FailureCauses};
use ranch_domain::jobs::{
    AddJobResult, BulkActionResult, JobActionResult, JobBatch, JobLogs, JobLogsQuery, JobPage,
    JobPageQuery, JobTarget, NewJob, RecentJobsQuery,
};
use ranch_domain::lock::ReadOnlyState;
use ranch_domain::maintenance::QueueMaintenance;
use ranch_domain::metrics::{MetricsRange, QueueMetrics, QueueMetricsQuery, Throughput};
use ranch_domain::probe::PingReport;
use ranch_domain::queues::{Queue, QueueActionResult, QueueId};
use ranch_domain::schedule::{SchedulerActionResult, SchedulerSpec, SchedulerTarget};
use ranch_domain::timeline::{WindowQuery, WindowSummary};

#[derive(Clone)]
pub struct ConnectionHandle {
    pub(super) tx: mpsc::Sender<ActorCommand>,
    pub(super) cancel: CancellationToken,
    /// Sequence of the latest job search / sort requested on
    /// this connection. A scan carries its own number and stops as soon as
    /// a newer one exists (the actor is serial: the newer request waits in
    /// the mailbox until the older scan lets go).
    pub(super) job_query_seq: Arc<AtomicU64>,
}

/// Lets a running scan ask "has a newer query been requested?".
#[derive(Debug, Clone)]
pub struct QueryTicket {
    latest: Arc<AtomicU64>,
    mine: u64,
}

impl QueryTicket {
    pub fn is_superseded(&self) -> bool {
        self.latest.load(Ordering::Relaxed) > self.mine
    }
}

#[derive(Debug, Error)]
pub enum ActorError {
    /// The actor's mailbox is closed (the task already ended). Caller
    /// should treat this as "connection no longer active".
    #[error("Connection actor unavailable (mailbox closed)")]
    ActorUnavailable,
    /// The actor accepted the message but never replied (it was shut
    /// down or the runtime is exiting). Treat as cancellation.
    #[error("Connection actor cancelled the request before reply")]
    Cancelled,
}

impl From<ActorError> for RanchError {
    fn from(e: ActorError) -> Self {
        RanchError::BackendUnavailable {
            reason: e.to_string(),
        }
    }
}

impl ConnectionHandle {
    /// Send a `Ping` and await the reply. Times out implicitly via the
    /// actor's own 2-second `tokio::time::timeout` on the redis op.
    pub async fn ping(&self) -> Result<PingReport, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::Ping { reply_to })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Discover queues + fetch counters in one round-trip
    /// pattern. The actor handles SCAN + per-queue MULTI/EXEC + schema
    /// caching internally. Inner `Result<Vec<Queue>, RanchError>` carries
    /// schema-not-recognized / redis errors; outer `Result<_, ActorError>`
    /// carries actor-level failures (mailbox closed, runtime exiting).
    pub async fn list_queues(&self) -> Result<Result<Vec<Queue>, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::ListQueues { reply_to })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Paginated job list for a (queue, state) pair. The
    /// actor handles LRANGE/ZRANGE + HGETALL pipeline + JobDecoder
    /// dispatch + BullMQ v6 marker filtering + per-job resilience.
    /// Same double-Result pattern as `list_queues`.
    pub async fn list_jobs(
        &self,
        args: JobPageQuery,
    ) -> Result<Result<JobPage, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        // Only scans take a ticket: plain range reads are cheap and must
        // never cancel (or be cancelled by) a search.
        let query = JobQuery {
            search: args.search.clone(),
            reason: args.reason.clone(),
            sort: args.sort,
        };
        let ticket = query.is_active().then(|| QueryTicket {
            mine: self.job_query_seq.fetch_add(1, Ordering::Relaxed) + 1,
            latest: Arc::clone(&self.job_query_seq),
        });
        self.tx
            .send(ActorCommand::ListJobs {
                args: Box::new(args),
                ticket,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Fetch + decode a single job by id (drill-down).
    /// Same double-Result pattern; inner `Err` is `JobNotFound` on empty
    /// hash or `SchemaNotRecognized` on cold-cache discovery failure.
    pub async fn get_job(&self, args: JobTarget) -> Result<Result<Job, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::GetJob {
                args: Box::new(args),
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Paginated read of BullMQ `Job.log()` entries.
    pub async fn get_job_logs(
        &self,
        args: JobLogsQuery,
    ) -> Result<Result<JobLogs, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::GetJobLogs {
                args: Box::new(args),
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// List CRON / repeatable schedulers for a queue.
    /// Same double-Result pattern; inner errors are `SchemaNotRecognized`
    /// on cold-cache discovery failure or `Redis` for transient issues.
    pub async fn list_cron_jobs(
        &self,
        queue_id: QueueId,
    ) -> Result<Result<Vec<CronJob>, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::ListCronJobs { queue_id, reply_to })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Worker counts of many queues (one `CLIENT LIST`).
    pub async fn count_queue_workers(
        &self,
        queue_ids: Vec<QueueId>,
    ) -> Result<Result<QueueWorkerCounts, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::CountQueueWorkers {
                queue_ids,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// BullMQ workers attached to a queue (`CLIENT LIST`).
    pub async fn list_workers(
        &self,
        queue_id: QueueId,
    ) -> Result<Result<QueueWorkers, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::ListWorkers { queue_id, reply_to })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Live events of the connection's queues.
    pub async fn read_queue_events(
        &self,
        args: EventsQuery,
    ) -> Result<Result<QueueEvents, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::ReadQueueEvents {
                args: Box::new(args),
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// The Redis server behind the connection.
    pub async fn get_server_info(
        &self,
    ) -> Result<Result<crate::redis_info::ServerInfo, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::GetServerInfo { reply_to })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Throughput of every queue of the connection.
    /// Flow links, added rates, oldest waiting jobs.
    pub async fn get_topology(
        &self,
    ) -> Result<Result<crate::bullmq_schema::topology::Topology, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::GetTopology { reply_to })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Follow (or stop following) the queues' event streams;
    /// whether they are followed now.
    pub async fn follow_events(
        &self,
        follow: bool,
        sink: Option<super::follow::EventSink>,
    ) -> Result<bool, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::FollowEvents {
                follow,
                sink,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// The summary of a Timeline window.
    pub async fn summarize_window(
        &self,
        args: WindowQuery,
    ) -> Result<Result<WindowSummary, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::SummarizeWindow {
                args: Box::new(args),
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    pub async fn get_throughput(
        &self,
        range: MetricsRange,
    ) -> Result<Result<Throughput, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::GetThroughput { range, reply_to })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// 24h observability metrics for a queue.
    pub async fn get_queue_metrics(
        &self,
        args: QueueMetricsQuery,
    ) -> Result<Result<QueueMetrics, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::GetQueueMetrics {
                args: Box::new(args),
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Failed jobs across every queue of the connection.
    pub async fn list_failed_jobs(
        &self,
        args: FailedJobsQuery,
    ) -> Result<Result<FailedJobsPage, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::ListFailedJobs {
                args: Box::new(args),
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Failed jobs grouped by cause.
    pub async fn list_failure_causes(
        &self,
        queue: Option<String>,
    ) -> Result<Result<FailureCauses, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::ListFailureCauses { queue, reply_to })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// The jobs that ran last, on one queue or across the connection.
    pub async fn list_recent_jobs(
        &self,
        args: RecentJobsQuery,
    ) -> Result<Result<RecentJobs, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::ListRecentJobs {
                args: Box::new(args),
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Read the actor's current in-memory lock state.
    /// `Result<ReadOnlyState, ActorError>`: the inner reply is
    /// infallible (no Redis IO) so only the mailbox/actor-runtime
    /// failure modes appear in the outer `Result`.
    pub async fn get_lock_state(&self) -> Result<ReadOnlyState, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::GetLockState { reply_to })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Flip the actor's lock state to `Unlocked`. Returns
    /// the new state on success. Double-`Result` shape (mailbox /
    /// runtime errors vs. audit-write errors), but today
    /// the inner result is always `Ok` since the audit write happens
    /// at the Tauri-command level; moving audit into the actor would
    /// use the inner error variant.
    pub async fn unlock(&self) -> Result<Result<ReadOnlyState, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::Unlock { reply_to })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Flip the actor's lock state to `Locked`. Symmetric
    /// to `unlock`.
    pub async fn lock(&self) -> Result<Result<ReadOnlyState, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::Lock { reply_to })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Retry a failed job into the waiting list. The
    /// actor handler routes through `check_lock_or_err` first and the
    /// audit log last (the write pipeline). Double-`Result` shape: outer
    /// for actor-runtime failures, inner for typed RanchErrors
    /// (`ConnectionLocked`, `InvalidJobState`, `JobNotFound`, ...).
    ///
    /// `caller` captures who initiated the action
    /// (UI vs. MCP agent + the agent identifier when applicable);
    /// flows through to the audit row.
    /// The flow a job belongs to.
    pub async fn get_flow(
        &self,
        args: JobTarget,
    ) -> Result<Result<FlowTree, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::GetFlow {
                args: Box::new(args),
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Flow roots found in a queue.
    pub async fn list_flow_roots(
        &self,
        queue_id: QueueId,
    ) -> Result<Result<FlowRoots, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::ListFlowRoots { queue_id, reply_to })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Affected count of a maintenance operation (read-only).
    pub async fn preview_queue_maintenance(
        &self,
        queue_id: QueueId,
        operation: QueueMaintenance,
    ) -> Result<Result<MaintenancePreview, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::PreviewQueueMaintenance {
                queue_id,
                operation,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Drain / clean / retry all failed, in batches.
    pub async fn run_queue_maintenance(
        &self,
        queue_id: QueueId,
        operation: QueueMaintenance,
        progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<Result<MaintenanceResult, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::RunQueueMaintenance {
                queue_id,
                operation,
                caller,
                progress,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    pub async fn add_job(
        &self,
        queue_id: QueueId,
        job: NewJob,
        caller: Caller,
    ) -> Result<Result<AddJobResult, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::AddJob {
                queue_id,
                job: Box::new(job),
                caller,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    pub async fn retry_job(
        &self,
        args: JobTarget,
        caller: Caller,
    ) -> Result<Result<JobActionResult, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::RetryJob {
                args: Box::new(args),
                caller,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Remove a job from every state container + the job
    /// hash + logs. Symmetric to `retry_job` regarding lock + audit.
    pub async fn remove_job(
        &self,
        args: JobTarget,
        caller: Caller,
    ) -> Result<Result<JobActionResult, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::RemoveJob {
                args: Box::new(args),
                caller,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Promote a delayed job into the waiting list.
    pub async fn promote_job(
        &self,
        args: JobTarget,
        caller: Caller,
    ) -> Result<Result<JobActionResult, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::PromoteJob {
                args: Box::new(args),
                caller,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Bulk single-queue retry. ONE lock check at the
    /// top of the handler, per-job MULTI/EXEC inside the actor's
    /// loop, ONE consolidated audit row at the end. Progress goes to
    /// `progress`.
    pub async fn bulk_retry_jobs(
        &self,
        args: JobBatch,
        progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<Result<BulkActionResult, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::BulkRetryJobs {
                args: Box::new(args),
                caller,
                progress,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Bulk single-queue remove.
    pub async fn bulk_remove_jobs(
        &self,
        args: JobBatch,
        progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<Result<BulkActionResult, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::BulkRemoveJobs {
                args: Box::new(args),
                caller,
                progress,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Bulk single-queue promote.
    pub async fn bulk_promote_jobs(
        &self,
        args: JobBatch,
        progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<Result<BulkActionResult, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::BulkPromoteJobs {
                args: Box::new(args),
                caller,
                progress,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Pause the queue.
    pub async fn pause_queue(
        &self,
        queue_id: QueueId,
        caller: Caller,
    ) -> Result<Result<QueueActionResult, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::PauseQueue {
                queue_id,
                caller,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Resume the queue.
    pub async fn resume_queue(
        &self,
        queue_id: QueueId,
        caller: Caller,
    ) -> Result<Result<QueueActionResult, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::ResumeQueue {
                queue_id,
                caller,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Create or replace a job scheduler.
    pub async fn upsert_scheduler(
        &self,
        queue_id: QueueId,
        spec: SchedulerSpec,
        caller: Caller,
    ) -> Result<Result<SchedulerSaved, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::UpsertScheduler {
                queue_id,
                spec: Box::new(spec),
                caller,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Delete a job scheduler.
    pub async fn remove_scheduler(
        &self,
        args: SchedulerTarget,
        caller: Caller,
    ) -> Result<Result<SchedulerActionResult, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::RemoveScheduler {
                args: Box::new(args),
                caller,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Pause a CRON / repeatable scheduler.
    pub async fn pause_scheduler(
        &self,
        args: SchedulerTarget,
        caller: Caller,
    ) -> Result<Result<SchedulerActionResult, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::PauseScheduler {
                args: Box::new(args),
                caller,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Resume a paused scheduler.
    pub async fn resume_scheduler(
        &self,
        args: SchedulerTarget,
        caller: Caller,
    ) -> Result<Result<SchedulerActionResult, RanchError>, ActorError> {
        let (reply_to, rx) = oneshot::channel();
        self.tx
            .send(ActorCommand::ResumeScheduler {
                args: Box::new(args),
                caller,
                reply_to,
            })
            .await
            .map_err(|_| ActorError::ActorUnavailable)?;
        rx.await.map_err(|_| ActorError::Cancelled)
    }

    /// Tell the actor to shut down. Returns immediately. The actor's
    /// task drives to completion within 1 second (the cancellation
    /// contract); the registry awaits the `JoinHandle` to confirm.
    ///
    /// Sending `Shutdown` on the mailbox is best-effort (the channel
    /// may be full or closed); we ALSO cancel the token directly so
    /// any in-flight redis op aborts even if the message never reaches
    /// the loop.
    pub fn signal_shutdown(&self) {
        let _ = self.tx.try_send(ActorCommand::Shutdown);
        self.cancel.cancel();
    }
}
