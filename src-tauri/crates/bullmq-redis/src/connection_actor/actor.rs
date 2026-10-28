//! `ConnectionActor` task body, command enum, and per-connection
//! runtime config.
//!
//! The actor itself is a `tokio::spawn`-able async function ([`run_actor`])
//! that listens on an `mpsc::Receiver<ActorCommand>` racing against a
//! [`tokio_util::sync::CancellationToken`]. Each `ActorCommand` variant
//! carries a `oneshot::Sender` for its reply. Reads run on their own task
//! ([`spawn_read`]); writes and lock changes run in the loop, in order.
//! The handlers live in `reads`, `writes` and `follow`.
//!
//! ## Cancellation contract
//!
//! Every Redis I/O the actor runs is wrapped in `tokio::select!` against
//! the cancellation token. On `Shutdown` the token fires; the actor's
//! loop drains pending messages with a sentinel "Cancelled" reply, then
//! exits. In-flight ops cancel within 1 second.
//!
//! ## Decoding
//!
//! BullMQ's key layout and field encoding live in `crate::bullmq_schema`;
//! the actor detects a queue's schema once and decodes through it.

use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::discovery::RegistrySlot;
use super::follow::{follow_events, EventSink};
use super::handle::QueryTicket;
use super::link::{handle_ping, Link};
use super::reads::events::handle_read_queue_events;
use super::reads::failed::{handle_list_failed_jobs, handle_list_failure_causes};
use super::reads::flows::{handle_get_flow, handle_list_flow_roots};
use super::reads::jobs::{
    handle_get_job, handle_get_job_logs, handle_list_jobs, handle_list_recent_jobs,
};
use super::reads::metrics::{
    handle_get_queue_metrics, handle_get_throughput, handle_summarize_window,
};
use super::reads::queues::handle_list_queues;
use super::reads::schedulers::handle_list_cron_jobs;
use super::reads::server::{handle_get_server_info, handle_get_topology};
use super::reads::workers::{handle_count_queue_workers, handle_list_workers};
use super::writes::add_job::handle_add_job;
use super::writes::bulk::{
    handle_bulk_promote_jobs, handle_bulk_remove_jobs, handle_bulk_retry_jobs,
};
use super::writes::job_actions::{handle_promote_job, handle_remove_job, handle_retry_job};
use super::writes::maintenance::{handle_preview_queue_maintenance, handle_run_queue_maintenance};
use super::writes::queue::{handle_pause_queue, handle_resume_queue};
use super::writes::schedulers::{
    handle_pause_scheduler, handle_remove_scheduler, handle_resume_scheduler,
    handle_upsert_scheduler,
};
use crate::bullmq_schema::flow::{FlowRoots, FlowTree};
use crate::bullmq_schema::{
    BullmqSchema, CronJob, Job, MaintenancePreview, MaintenanceResult, QueueWorkerCounts,
    QueueWorkers, RecentJobs, SchedulerSaved,
};
use crate::endpoint::RedisEndpoint;
use ranch_application::ports::ProgressSink;
use ranch_application::ports::SecretStore;
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
use ranch_domain::probe::{PingReport, ProbeError};
use ranch_domain::queues::{Queue, QueueActionResult, QueueId};
use ranch_domain::schedule::{SchedulerActionResult, SchedulerSpec, SchedulerTarget};
use ranch_domain::timeline::{WindowQuery, WindowSummary};

/// Per-connection runtime config. Resolved at spawn time from
/// (SQLite row + keychain credential). Internal to the actor module,
/// not a wire type, not specta-derived.
///
/// **NOTE**: contains the SSH/Redis password in plaintext for the
/// actor's lifetime. This is acceptable per architecture (the trust
/// boundary is OS-level user isolation; the keychain protects only
/// at-rest credentials, not in-process). Password lifetime is bounded
/// by actor lifetime: `remove_connection` shuts the actor down,
/// dropping the config.
#[derive(Clone)]
pub struct ConnectionConfig {
    /// Where Redis is, as the BullMQ backend reads the saved endpoint.
    pub endpoint: RedisEndpoint,
    pub password: Option<String>,
    /// Initial lock state from the `connections.is_locked`
    /// column at the time the actor spawns. The actor stores this in
    /// `ActorState.lock_state` and never reads `is_locked` again for
    /// its lifetime. Restart = re-spawn = re-read = back to this value
    /// (unlock is in-memory only).
    pub initial_lock_state: ReadOnlyState,
    /// Connection id, used to populate the
    /// `RanchError::ConnectionLocked` payload and the audit-log
    /// snapshot fields for unlock/lock toggles.
    pub connection_id: String,
    /// Connection display name. Same role as
    /// `connection_id`: feeds the error payload + audit-log snapshot.
    pub connection_name: String,
    /// Env color token (e.g. `"env-production"`). Snapshot
    /// for audit-log row stripes. `None` is tolerated for the rare
    /// case where the row has no env: the audit row keeps a NULL.
    pub env_color: Option<String>,
    /// Authorizes and audits this connection's writes.
    pub writes: Arc<ranch_application::WritePipeline>,
    /// SSH password fetched from the keychain at
    /// `resolve_connection_config` time. Only meaningful when
    /// `endpoint.ssh_tunnel.auth.kind == "password"`. None otherwise. Same
    /// trust-boundary stance as `password`: plaintext in-process for
    /// the actor's lifetime, dropped on shutdown.
    pub ssh_password: Option<String>,
    /// Keychain handle for the SSH tunnel's TOFU host-key
    /// pin (`{id}::ssh::hostkey`). Cloning the Arc is a cheap refcount
    /// bump; the underlying adapter is `Send + Sync`.
    pub keychain: Arc<dyn SecretStore>,
}

// Manual Debug impl: `dyn SecretStore` doesn't impl Debug, so we
// skip that field. Useful only for tracing/log messages anyway.
impl ConnectionConfig {
    /// The connection as errors and audit rows name it.
    pub fn stamp(&self) -> ranch_application::ConnectionStamp {
        ranch_application::ConnectionStamp {
            id: self.connection_id.clone(),
            name: self.connection_name.clone(),
            env_color: self.env_color.clone(),
        }
    }
}

impl std::fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionConfig")
            .field("endpoint", &self.endpoint)
            .field("connection_id", &self.connection_id)
            .field("connection_name", &self.connection_name)
            .field("env_color", &self.env_color)
            .finish_non_exhaustive()
    }
}

/// Internal command type: what the registry sends across the mailbox.
/// Each variant carries a `oneshot::Sender` for its reply. Cancellation
/// is by the actor's `CancellationToken` firing.
pub(super) enum ActorCommand {
    Ping {
        reply_to: oneshot::Sender<PingReport>,
    },
    /// Discover queues + fetch counters in one go.
    ListQueues {
        reply_to: oneshot::Sender<Result<Vec<Queue>, RanchError>>,
    },
    /// Paginated job list for a (queue, state) pair.
    ListJobs {
        args: Box<JobPageQuery>,
        /// Set for search / sort scans: lets the scan stop when
        /// a newer one has been requested.
        ticket: Option<QueryTicket>,
        reply_to: oneshot::Sender<Result<JobPage, RanchError>>,
    },
    /// Fetch + decode a single job by id (drill-down view).
    GetJob {
        args: Box<JobTarget>,
        reply_to: oneshot::Sender<Result<Job, RanchError>>,
    },
    /// Paginated read of BullMQ `Job.log()` entries from
    /// the `bull:{q}:{id}:logs` list.
    GetJobLogs {
        args: Box<JobLogsQuery>,
        reply_to: oneshot::Sender<Result<JobLogs, RanchError>>,
    },
    /// List of CRON / repeatable schedulers for a queue.
    /// Reads `bull:{q}:repeat` (ZSET) + per-scheduler `bull:{q}:repeat:{key}`
    /// hashes via a non-atomic pipeline.
    ListCronJobs {
        queue_id: QueueId,
        reply_to: oneshot::Sender<Result<Vec<CronJob>, RanchError>>,
    },
    /// 24h observability metrics for a queue: per-hour
    /// `completed` + `failed` counts (via `ZCOUNT` × 48), plus average
    /// processing time and queue delay over the last 100 completed
    /// jobs (via `ZRANGEBYSCORE` + `HMGET`).
    GetQueueMetrics {
        args: Box<QueueMetricsQuery>,
        reply_to: oneshot::Sender<Result<QueueMetrics, RanchError>>,
    },
    /// Live events of the connection's queues.
    ReadQueueEvents {
        args: Box<EventsQuery>,
        reply_to: oneshot::Sender<Result<QueueEvents, RanchError>>,
    },
    /// The Redis server (version, latency, memory, ops/s).
    GetServerInfo {
        reply_to: oneshot::Sender<Result<crate::redis_info::ServerInfo, RanchError>>,
    },
    /// Flow links, added rates, oldest waiting jobs.
    /// Follow (or stop) the queues' event streams; replies
    /// whether they are followed now.
    FollowEvents {
        follow: bool,
        sink: Option<EventSink>,
        reply_to: oneshot::Sender<bool>,
    },
    GetTopology {
        reply_to: oneshot::Sender<Result<crate::bullmq_schema::topology::Topology, RanchError>>,
    },
    /// The summary of a Timeline window.
    SummarizeWindow {
        args: Box<WindowQuery>,
        reply_to: oneshot::Sender<Result<WindowSummary, RanchError>>,
    },
    /// Throughput of every queue, native metrics only.
    GetThroughput {
        range: MetricsRange,
        reply_to: oneshot::Sender<Result<Throughput, RanchError>>,
    },
    /// BullMQ workers attached to a queue, from `CLIENT LIST`
    /// (see `bullmq_schema::workers`). A refusal is cached per connection.
    ListWorkers {
        queue_id: QueueId,
        reply_to: oneshot::Sender<Result<QueueWorkers, RanchError>>,
    },
    /// Worker counts of many queues from one `CLIENT LIST`.
    CountQueueWorkers {
        queue_ids: Vec<QueueId>,
        reply_to: oneshot::Sender<Result<QueueWorkerCounts, RanchError>>,
    },
    /// Failed jobs across every queue of the connection,
    /// newest first (bounded scan; see `domain::failures`).
    ListFailedJobs {
        args: Box<FailedJobsQuery>,
        reply_to: oneshot::Sender<Result<FailedJobsPage, RanchError>>,
    },
    /// Failed jobs grouped by cause.
    ListFailureCauses {
        queue: Option<String>,
        reply_to: oneshot::Sender<Result<FailureCauses, RanchError>>,
    },
    /// The jobs that ran last: one queue, or the whole connection merged
    /// (bounded; see `bullmq_schema::recent`).
    ListRecentJobs {
        args: Box<RecentJobsQuery>,
        reply_to: oneshot::Sender<Result<RecentJobs, RanchError>>,
    },
    /// Read the current in-memory lock state. No Redis IO.
    /// `unlock_connection` / `lock_connection` commands mutate via
    /// `Unlock` / `Lock` below; this is a pure read so the UI can
    /// hydrate the `ProductionLockBanner` on mount.
    GetLockState {
        reply_to: oneshot::Sender<ReadOnlyState>,
    },
    /// Flip the actor's `lock_state` to `Unlocked`. The audit entry is
    /// written by the caller (`commands::safety`), which holds the pool.
    Unlock {
        reply_to: oneshot::Sender<Result<ReadOnlyState, RanchError>>,
    },
    /// Flip the actor's `lock_state` to `Locked`. Symmetric to `Unlock`.
    Lock {
        reply_to: oneshot::Sender<Result<ReadOnlyState, RanchError>>,
    },
    /// The flow (parent / child tree) a job belongs to.
    GetFlow {
        args: Box<JobTarget>,
        reply_to: oneshot::Sender<Result<FlowTree, RanchError>>,
    },
    /// Flow roots found in a queue (bounded discovery).
    ListFlowRoots {
        queue_id: QueueId,
        reply_to: oneshot::Sender<Result<FlowRoots, RanchError>>,
    },
    /// How many jobs a maintenance operation would touch. Read-only.
    PreviewQueueMaintenance {
        queue_id: QueueId,
        operation: QueueMaintenance,
        reply_to: oneshot::Sender<Result<MaintenancePreview, RanchError>>,
    },
    /// Drain / clean / retry all failed: batched Lua scripts
    /// (`bullmq_schema::maintenance`), progress events, one audit row.
    RunQueueMaintenance {
        queue_id: QueueId,
        operation: QueueMaintenance,
        caller: Caller,
        progress: Arc<dyn ProgressSink>,
        reply_to: oneshot::Sender<Result<MaintenanceResult, RanchError>>,
    },
    /// Enqueue a job: one atomic Lua script (see
    /// `bullmq_schema::add_job`). Gated + audited like every other write.
    AddJob {
        queue_id: QueueId,
        job: Box<NewJob>,
        caller: Caller,
        reply_to: oneshot::Sender<Result<AddJobResult, RanchError>>,
    },
    /// single-job destructive: retry a failed job back
    /// into the waiting list. Gated by `check_lock_or_err`; audited
    /// through the write pipeline (success OR failure). It
    /// carries `Caller` so MCP callers can record
    /// `source = Agent` + the agent client identifier.
    RetryJob {
        args: Box<JobTarget>,
        caller: Caller,
        reply_to: oneshot::Sender<Result<JobActionResult, RanchError>>,
    },
    /// single-job destructive: remove a job from every
    /// state container + the job hash + logs.
    RemoveJob {
        args: Box<JobTarget>,
        caller: Caller,
        reply_to: oneshot::Sender<Result<JobActionResult, RanchError>>,
    },
    /// single-job destructive: promote a delayed job
    /// into the waiting list.
    PromoteJob {
        args: Box<JobTarget>,
        caller: Caller,
        reply_to: oneshot::Sender<Result<JobActionResult, RanchError>>,
    },
    /// Bulk single-queue retry. ONE lock check at the
    /// top, per-job MULTI/EXEC inside a loop, ONE consolidated audit
    /// row at the end. Progress reported to `progress` every
    /// `BULK_PROGRESS_CHUNK` jobs.
    BulkRetryJobs {
        args: Box<JobBatch>,
        caller: Caller,
        progress: Arc<dyn ProgressSink>,
        reply_to: oneshot::Sender<Result<BulkActionResult, RanchError>>,
    },
    /// Bulk single-queue remove. Same shape as
    /// `BulkRetryJobs`.
    BulkRemoveJobs {
        args: Box<JobBatch>,
        caller: Caller,
        progress: Arc<dyn ProgressSink>,
        reply_to: oneshot::Sender<Result<BulkActionResult, RanchError>>,
    },
    /// Bulk single-queue promote. Same shape as
    /// `BulkRetryJobs`.
    BulkPromoteJobs {
        args: Box<JobBatch>,
        caller: Caller,
        progress: Arc<dyn ProgressSink>,
        reply_to: oneshot::Sender<Result<BulkActionResult, RanchError>>,
    },
    /// Pause the queue's `:wait` list (rename to `:paused`)
    /// + flip the `:meta` `paused` flag. Idempotent.
    PauseQueue {
        queue_id: QueueId,
        caller: Caller,
        reply_to: oneshot::Sender<Result<QueueActionResult, RanchError>>,
    },
    /// Symmetric to `PauseQueue`: rename `:paused` back
    /// to `:wait` + clear the `:meta` flag.
    ResumeQueue {
        queue_id: QueueId,
        caller: Caller,
        reply_to: oneshot::Sender<Result<QueueActionResult, RanchError>>,
    },
    /// Pause a CRON / repeatable scheduler by removing
    /// its entry from the `:repeat` ZSET + flagging the per-scheduler
    /// hash. Ranch-original (BullMQ has no public pause API for
    /// repeatables).
    PauseScheduler {
        args: Box<SchedulerTarget>,
        caller: Caller,
        reply_to: oneshot::Sender<Result<SchedulerActionResult, RanchError>>,
    },
    /// Create or replace a job scheduler (`upsert_scheduler.lua`).
    UpsertScheduler {
        queue_id: QueueId,
        spec: Box<SchedulerSpec>,
        caller: Caller,
        reply_to: oneshot::Sender<Result<SchedulerSaved, RanchError>>,
    },
    /// Delete a job scheduler and its pending job.
    RemoveScheduler {
        args: Box<SchedulerTarget>,
        caller: Caller,
        reply_to: oneshot::Sender<Result<SchedulerActionResult, RanchError>>,
    },
    /// re-arm a paused scheduler with the current
    /// timestamp as the next-run score.
    ResumeScheduler {
        args: Box<SchedulerTarget>,
        caller: Caller,
        reply_to: oneshot::Sender<Result<SchedulerActionResult, RanchError>>,
    },
    Shutdown,
}

/// Per-actor mutable state. Today: connection config (immutable) +
/// detected BullMQ schema (set once on first successful detection,
/// never invalidated) + per-connection read-only lock state
/// + cached SSH tunnel handle.
///
/// `Clone`: a read runs on its own copy, concurrently with the
/// others. What must be shared lives behind an `Arc` (the connection and
/// tunnel, the schema cache, the queue registry, the CLIENT LIST refusal);
/// `lock_state` is copied, as writes and lock changes stay in the loop.
#[derive(Clone)]
pub(super) struct ActorState {
    pub(super) cfg: ConnectionConfig,
    /// Cached BullMQ schema. None until first successful detection.
    /// Re-attempted on each `ListQueues` call until cached, then
    /// frozen for the actor's lifetime; see module docs for the
    /// invalidation rationale (v1: re-add the connection to refresh).
    pub(super) cached_schema: Arc<std::sync::Mutex<Option<BullmqSchema>>>,
    /// read-only lock state. Initialized from
    /// `cfg.initial_lock_state` at spawn (which the registry derived
    /// from the `connections.is_locked` column). Mutated only via
    /// `ActorCommand::Unlock` / `ActorCommand::Lock`. In-memory only:
    /// restart re-locks.
    pub(super) lock_state: ReadOnlyState,
    /// Cached SSH tunnel handle. `None` for non-SSH
    /// connections AND when the tunnel has not yet been opened. The
    /// first Redis op lazily establishes the tunnel + stores it here;
    /// subsequent ops reuse. Invalidated to `None` on Redis-side
    /// failure (the failure may be on the SSH side; next call
    /// re-tunnels). Cleanup is implicit: drop on actor shutdown fires
    /// the tunnel's Drop impl, which closes the SSH session.
    /// With the persistent connection, in [`Link`]: shared by
    /// concurrent reads, and locked while one of them opens it.
    pub(super) link: Arc<tokio::sync::Mutex<Link>>,
    /// Capability probe result for `CLIENT LIST`. `Some(reason)`
    /// once this Redis has refused it (managed providers, ACLs): later calls
    /// answer "unsupported" without asking again. `None` = not refused so
    /// far. Lives as long as the actor; editing the connection respawns it.
    pub(super) client_list_denied: Arc<std::sync::Mutex<Option<String>>>,
    /// The last `CLIENT LIST` reply and when it arrived, locked
    /// while one is read: the Workers tab asks once per queue at the same
    /// moment, and they all share one read.
    pub(super) client_list_cache: Arc<tokio::sync::Mutex<Option<(std::time::Instant, String)>>>,
    /// The queues found by the last SCAN, shared with the task
    /// that refreshes them in the background.
    pub(super) queue_registry: Arc<std::sync::Mutex<RegistrySlot>>,
}

/// Reads one connection runs at once; more wait for a slot.
const MAX_CONCURRENT_READS: usize = 16;

/// Run a read on its own task, so a slow one never holds up a
/// ping, a click or another poll. Skipped when its caller has
/// already given up, and stopped when the caller goes away mid-way.
/// Writes stay in the actor loop, in order.
fn spawn_read<T: Send + 'static>(
    mut reply_to: oneshot::Sender<T>,
    read: impl std::future::Future<Output = T> + Send + 'static,
) {
    if reply_to.is_closed() {
        return;
    }
    tokio::spawn(async move {
        let result = tokio::select! {
            r = read => Some(r),
            _ = reply_to.closed() => None,
        };
        if let Some(r) = result {
            let _ = reply_to.send(r);
        }
    });
}

/// A shared cache's value; a panic elsewhere never makes it unusable.
pub(super) fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Actor task body. Spawned via `tokio::spawn(run_actor(rx, token, cfg))`
/// from [`ConnectionRegistry::get_or_spawn`](super::registry::ConnectionRegistry).
pub(super) async fn run_actor(
    mut rx: mpsc::Receiver<ActorCommand>,
    token: CancellationToken,
    cfg: ConnectionConfig,
) {
    let lock_state = cfg.initial_lock_state;
    let mut state = ActorState {
        cfg,
        cached_schema: Arc::default(),
        lock_state,
        link: Arc::default(),
        client_list_denied: Arc::default(),
        client_list_cache: Arc::default(),
        queue_registry: Arc::default(),
    };
    // Reads in flight at once on this connection.
    let reads = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_READS));
    // Windows following the event streams, and the reader.
    let mut followers = 0usize;
    let mut follower: Option<CancellationToken> = None;
    loop {
        tokio::select! {
            biased;
            // Prefer cancellation over message recv on each iteration so
            // a Shutdown signal that arrives during a long handler doesn't
            // get queued behind another message.
            _ = token.cancelled() => break,
            msg = rx.recv() => {
                let Some(msg) = msg else { break }; // mailbox closed
                match msg {
                    ActorCommand::Ping { reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_ping(&mut st, &tk).await
                        });
                    }
                    ActorCommand::ListQueues { reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_list_queues(&mut st, &tk).await
                        });
                    }
                    ActorCommand::ListJobs { args, ticket, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_list_jobs(&mut st, &args, ticket.as_ref(), &tk).await
                        });
                    }
                    ActorCommand::GetJob { args, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_get_job(&mut st, &args, &tk).await
                        });
                    }
                    ActorCommand::GetJobLogs { args, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_get_job_logs(&mut st, &args, &tk).await
                        });
                    }
                    ActorCommand::ListCronJobs { queue_id, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_list_cron_jobs(&mut st, &queue_id, &tk).await
                        });
                    }
                    ActorCommand::GetQueueMetrics { args, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_get_queue_metrics(&mut st, &args, &tk).await
                        });
                    }
                    ActorCommand::ReadQueueEvents { args, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_read_queue_events(&mut st, &args, &tk).await
                        });
                    }
                    ActorCommand::GetServerInfo { reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_get_server_info(&mut st, &tk).await
                        });
                    }
                    ActorCommand::GetThroughput { range, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_get_throughput(&mut st, range, &tk).await
                        });
                    }
                    ActorCommand::GetTopology { reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_get_topology(&mut st, &tk).await
                        });
                    }
                    ActorCommand::SummarizeWindow { args, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_summarize_window(&mut st, &args, &tk).await
                        });
                    }
                    ActorCommand::ListWorkers { queue_id, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        let asked_at = std::time::Instant::now();
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_list_workers(&mut st, &queue_id, asked_at, &tk).await
                        });
                    }
                    ActorCommand::CountQueueWorkers { queue_ids, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        let asked_at = std::time::Instant::now();
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_count_queue_workers(&mut st, &queue_ids, asked_at, &tk).await
                        });
                    }
                    ActorCommand::ListFailedJobs { args, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_list_failed_jobs(&mut st, &args, &tk).await
                        });
                    }
                    ActorCommand::ListFailureCauses { queue, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_list_failure_causes(&mut st, queue.as_deref(), &tk).await
                        });
                    }
                    ActorCommand::ListRecentJobs { args, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_list_recent_jobs(&mut st, &args, &tk).await
                        });
                    }
                    ActorCommand::GetLockState { reply_to } => {
                        // Pure in-memory read, no IO,
                        // no cancellation race.
                        let _ = reply_to.send(state.lock_state);
                    }
                    ActorCommand::Unlock { reply_to } => {
                        // Flip in-memory state to Unlocked
                        // and reply with the new state. The audit-log
                        // write is performed by the Tauri command
                        // (`commands/safety.rs::unlock_connection_inner`)
                        // after the actor returns; that's the only
                        // call site with access to the SqlitePool, and
                        // toggle ops have no Redis-side step to record
                        // success/failure for.
                        state.lock_state = ReadOnlyState::Unlocked;
                        let _ = reply_to.send(Ok(state.lock_state));
                    }
                    ActorCommand::Lock { reply_to } => {
                        // Symmetric to Unlock above.
                        state.lock_state = ReadOnlyState::Locked;
                        let _ = reply_to.send(Ok(state.lock_state));
                    }
                    ActorCommand::GetFlow { args, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_get_flow(&mut st, &args, &tk).await
                        });
                    }
                    ActorCommand::ListFlowRoots { queue_id, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_list_flow_roots(&mut st, &queue_id, &tk).await
                        });
                    }
                    ActorCommand::PreviewQueueMaintenance { queue_id, operation, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_preview_queue_maintenance(&mut st, &queue_id, &operation, &tk).await
                        });
                    }
                    ActorCommand::RunQueueMaintenance { queue_id, operation, caller, progress, reply_to } => {
                        let result = handle_run_queue_maintenance(&mut state, &queue_id, &operation, &caller, progress.as_ref(), &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::AddJob { queue_id, job, caller, reply_to } => {
                        let result = handle_add_job(&mut state, &queue_id, &job, &caller, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::RetryJob { args, caller, reply_to } => {
                        let result = handle_retry_job(&mut state, &args, &caller, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::RemoveJob { args, caller, reply_to } => {
                        let result = handle_remove_job(&mut state, &args, &caller, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::PromoteJob { args, caller, reply_to } => {
                        let result = handle_promote_job(&mut state, &args, &caller, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::BulkRetryJobs { args, caller, progress, reply_to } => {
                        let result = handle_bulk_retry_jobs(&mut state, &args, &caller, progress.as_ref(), &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::BulkRemoveJobs { args, caller, progress, reply_to } => {
                        let result = handle_bulk_remove_jobs(&mut state, &args, &caller, progress.as_ref(), &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::BulkPromoteJobs { args, caller, progress, reply_to } => {
                        let result = handle_bulk_promote_jobs(&mut state, &args, &caller, progress.as_ref(), &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::PauseQueue { queue_id, caller, reply_to } => {
                        let result = handle_pause_queue(&mut state, &queue_id, &caller, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::ResumeQueue { queue_id, caller, reply_to } => {
                        let result = handle_resume_queue(&mut state, &queue_id, &caller, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::UpsertScheduler { queue_id, spec, caller, reply_to } => {
                        let result = handle_upsert_scheduler(&mut state, &queue_id, &spec, &caller, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::RemoveScheduler { args, caller, reply_to } => {
                        let result = handle_remove_scheduler(&mut state, &args, &caller, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::PauseScheduler { args, caller, reply_to } => {
                        let result = handle_pause_scheduler(&mut state, &args, &caller, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::ResumeScheduler { args, caller, reply_to } => {
                        let result = handle_resume_scheduler(&mut state, &args, &caller, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::FollowEvents { follow, sink, reply_to } => {
                        if follow {
                            followers += 1;
                        } else {
                            followers = followers.saturating_sub(1);
                        }
                        if followers == 0 {
                            if let Some(reader) = follower.take() {
                                reader.cancel();
                            }
                        } else if follower.is_none() {
                            if let Some(sink) = sink {
                                let reader = token.child_token();
                                // Replies once it knows where to start: what
                                // happens after the caller hears back is seen.
                                tokio::spawn(follow_events(
                                    state.clone(),
                                    sink,
                                    reader.clone(),
                                    Some(reply_to),
                                ));
                                follower = Some(reader);
                                continue;
                            }
                        }
                        let _ = reply_to.send(follower.is_some());
                    }
                    ActorCommand::Shutdown => {
                        token.cancel();
                        break;
                    }
                }
            }
        }
    }
    // Drain remaining messages so callers awaiting their oneshot don't
    // hang. Each pending message gets a sentinel "actor shut down" reply.
    while let Ok(msg) = rx.try_recv() {
        match msg {
            ActorCommand::Ping { reply_to } => {
                let _ = reply_to.send(actor_cancelled_result());
            }
            ActorCommand::ListQueues { reply_to } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::ListJobs { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::GetJob { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::GetJobLogs { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::ListCronJobs { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::GetQueueMetrics { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::ReadQueueEvents { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::GetServerInfo { reply_to } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::GetThroughput { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::FollowEvents { reply_to, .. } => {
                let _ = reply_to.send(false);
            }
            ActorCommand::GetTopology { reply_to } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::SummarizeWindow { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::ListFailedJobs { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::ListFailureCauses { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::ListRecentJobs { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::ListWorkers { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::CountQueueWorkers { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::GetLockState { reply_to } => {
                // Best-effort: caller awaiting the lock state during
                // drain gets the actor's last-known state. The reply
                // type carries no error variant, so we send the
                // current value and let the caller move on.
                let _ = reply_to.send(state.lock_state);
            }
            ActorCommand::Unlock { reply_to } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::Lock { reply_to } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::AddJob { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::GetFlow { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::ListFlowRoots { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::PreviewQueueMaintenance { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::RunQueueMaintenance { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::RetryJob { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::RemoveJob { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::PromoteJob { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::BulkRetryJobs { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::BulkRemoveJobs { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::BulkPromoteJobs { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::PauseQueue { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::ResumeQueue { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::UpsertScheduler { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::RemoveScheduler { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::PauseScheduler { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::ResumeScheduler { reply_to, .. } => {
                let _ = reply_to.send(Err(RanchError::InvalidInput {
                    reason: "Connection actor shut down".to_string(),
                }));
            }
            ActorCommand::Shutdown => {}
        }
    }
}

/// Sentinel reply when the actor is shutting down, as a
/// `ProbeError::Other`.
pub(super) fn actor_cancelled_result() -> PingReport {
    PingReport {
        success: false,
        latency_ms: None,
        error: Some(ProbeError::Other {
            reason: "Connection actor shut down".to_string(),
        }),
    }
}
