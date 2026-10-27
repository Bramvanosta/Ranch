//! Registry of live `ConnectionActor` tasks, keyed by
//! `connection_id`. Stored on `AppState` (wrapped in `Arc`) so all Tauri
//! commands and (future) MCP tools share the same registry — concurrent
//! calls against the same connection serialize through the actor's
//! mailbox.
//!
//! ## Lifecycle
//!
//! - **Spawn**: lazy. First call to `ConnectionRegistry::ping(id, cfg)`
//!   for an unknown id spins up the actor and inserts a handle.
//! - **Reuse**: subsequent calls find the existing handle.
//! - **Shutdown**: explicit via [`ConnectionRegistry::shutdown`] (called
//!   from `remove_connection`) OR registry drop at app exit.
//!
//! ## Concurrent spawn safety
//!
//! Two callers reaching `get_or_spawn` at the same time both fall to the
//! slow path. After spawning, we re-check the map under the lock; if
//! another caller won the insert, we signal-shutdown the just-spawned
//! actor and return the existing handle. The wasted spawn is < 1ms.

use std::collections::HashMap;

use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::actor::{run_actor, ActorCommand, ConnectionConfig};
use super::handle::{ActorError, ConnectionHandle};
use crate::commands::connections::PingConnectionResult;
use crate::commands::jobs::{
    AddJobArgs, AddJobResult, BulkActionArgs, BulkActionResult, GetJobArgs, GetJobLogsArgs,
    JobActionArgs, JobActionResult, JobLogsOutput, ListFailedJobsArgs, ListFailureCausesArgs,
    ListJobsArgs, ListJobsOutput, ListRecentJobsArgs,
};
use crate::commands::queues::{
    CountQueueWorkersArgs, GetQueueMetricsArgs, GetThroughputArgs, ListCronJobsArgs,
    ListWorkersArgs, QueueActionArgs, QueueActionResult, QueueMaintenanceArgs, ReadQueueEventsArgs,
    SchedulerActionArgs, SchedulerActionResult, UpsertSchedulerArgs,
};
use crate::core::bullmq_schema::{
    CronJob, FailedJobsPage, FailureCauses, FlowRoots, FlowTree, Job, MaintenancePreview,
    MaintenanceResult, Queue, QueueMetrics, QueueWorkerCounts, QueueWorkers, RecentJobs,
    SchedulerSaved,
};
use crate::core::errors::RanchError;
use crate::core::safety_lock::ReadOnlyState;

#[derive(Default)]
pub struct ConnectionRegistry {
    inner: Mutex<HashMap<String, RegistryEntry>>,
}

struct RegistryEntry {
    handle: ConnectionHandle,
    join: JoinHandle<()>,
}

impl ConnectionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve (spawning if absent) the actor for `connection_id` and
    /// send a ping. The config is used ONLY when the actor is being
    /// spawned for the first time; subsequent calls reuse the actor's
    /// already-resolved client.
    ///
    /// **TODO (post-MVP)**: when credentials rotate (user runs
    /// `update_connection` with a new password), the registry will
    /// need a `refresh(id, cfg)` method to drop + respawn. The registry
    /// ships only the lazy-spawn + shutdown paths.
    pub async fn ping(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
    ) -> Result<PingConnectionResult, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.ping().await
    }

    /// Discover queues + fetch counters via the actor.
    /// Lazy-spawns the actor on first call (same lifecycle as `ping`).
    /// The double-Result is intentional: outer carries actor-level
    /// failures (mailbox closed); inner carries schema/redis failures
    /// the UI surfaces verbatim.
    pub async fn list_queues(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
    ) -> Result<Result<Vec<Queue>, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.list_queues().await
    }

    /// Paginated job list for a (queue, state) pair.
    /// Same lazy-spawn + double-Result pattern as `list_queues`.
    pub async fn list_jobs(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: ListJobsArgs,
    ) -> Result<Result<ListJobsOutput, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.list_jobs(args).await
    }

    /// Fetch a single job for the drill-down view.
    pub async fn get_job(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: GetJobArgs,
    ) -> Result<Result<Job, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.get_job(args).await
    }

    /// Read paginated BullMQ `Job.log()` entries.
    pub async fn get_job_logs(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: GetJobLogsArgs,
    ) -> Result<Result<JobLogsOutput, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.get_job_logs(args).await
    }

    /// List CRON / repeatable schedulers for a queue.
    pub async fn list_cron_jobs(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: ListCronJobsArgs,
    ) -> Result<Result<Vec<CronJob>, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.list_cron_jobs(args).await
    }

    /// Worker counts of many queues, one `CLIENT LIST`.
    pub async fn count_queue_workers(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: CountQueueWorkersArgs,
    ) -> Result<Result<QueueWorkerCounts, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.count_queue_workers(args).await
    }

    /// BullMQ workers attached to a queue.
    pub async fn list_workers(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: ListWorkersArgs,
    ) -> Result<Result<QueueWorkers, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.list_workers(args).await
    }

    /// Live events of the connection's queues.
    pub async fn read_queue_events(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: ReadQueueEventsArgs,
    ) -> Result<Result<crate::core::bullmq_schema::events::QueueEvents, RanchError>, ActorError>
    {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.read_queue_events(args).await
    }

    /// The Redis server behind the connection.
    pub async fn get_server_info(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
    ) -> Result<Result<crate::core::redis_info::ServerInfo, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.get_server_info().await
    }

    /// Throughput of every queue of the connection.
    pub async fn get_throughput(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: GetThroughputArgs,
    ) -> Result<Result<crate::core::bullmq_schema::Throughput, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.get_throughput(args).await
    }

    /// Flow links, added rates, oldest waiting jobs.
    pub async fn get_topology(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
    ) -> Result<Result<crate::core::bullmq_schema::topology::Topology, RanchError>, ActorError>
    {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.get_topology().await
    }

    /// Follow (or stop following) the queues' event streams.
    pub async fn follow_events(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        follow: bool,
        sink: Option<super::actor::EventSink>,
    ) -> Result<bool, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.follow_events(follow, sink).await
    }

    /// The summary of a Timeline window.
    pub async fn summarize_window(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: crate::commands::queues::SummarizeWindowArgs,
    ) -> Result<Result<crate::core::bullmq_schema::timeline::WindowSummary, RanchError>, ActorError>
    {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.summarize_window(args).await
    }

    /// 24h observability metrics for a queue.
    pub async fn get_queue_metrics(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: GetQueueMetricsArgs,
    ) -> Result<Result<QueueMetrics, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.get_queue_metrics(args).await
    }

    /// Failed jobs across every queue of the connection.
    pub async fn list_failed_jobs(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: ListFailedJobsArgs,
    ) -> Result<Result<FailedJobsPage, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.list_failed_jobs(args).await
    }

    /// Failed jobs grouped by cause.
    pub async fn list_failure_causes(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: ListFailureCausesArgs,
    ) -> Result<Result<FailureCauses, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.list_failure_causes(args).await
    }

    /// The jobs that ran last, on one queue or across the connection.
    pub async fn list_recent_jobs(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: ListRecentJobsArgs,
    ) -> Result<Result<RecentJobs, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.list_recent_jobs(args).await
    }

    /// Read the actor's current in-memory lock state.
    /// Lazy-spawns the actor on first call (mirroring `ping`), which
    /// initializes the state from `config.initial_lock_state` (derived
    /// from the `connections.is_locked` column by the calling Tauri
    /// command). Subsequent calls return the actor's mutated state if
    /// the user has unlocked / re-locked in the meantime.
    pub async fn get_lock_state(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
    ) -> Result<ReadOnlyState, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.get_lock_state().await
    }

    /// Flip the actor's lock state to `Unlocked`.
    /// Lazy-spawns on first call. Returns the new state on success.
    /// The audit-log entry is written by the Tauri command after this
    /// returns (the actor has no SQLite pool access).
    pub async fn unlock(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
    ) -> Result<Result<ReadOnlyState, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.unlock().await
    }

    /// Flip the actor's lock state to `Locked`. Symmetric
    /// to `unlock` — see that doc for details.
    pub async fn lock(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
    ) -> Result<Result<ReadOnlyState, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.lock().await
    }

    /// Retry a failed job. Lazy-spawn + double-Result shape.
    /// Inner errors include `ConnectionLocked` (from `check_lock_or_err`)
    /// and `InvalidJobState` (when the job isn't in `failed`).
    /// The flow a job belongs to.
    pub async fn get_flow(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: GetJobArgs,
    ) -> Result<Result<FlowTree, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.get_flow(args).await
    }

    /// Flow roots found in a queue.
    pub async fn list_flow_roots(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: QueueActionArgs,
    ) -> Result<Result<FlowRoots, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.list_flow_roots(args).await
    }

    /// Affected count of a maintenance operation.
    pub async fn preview_queue_maintenance(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: QueueMaintenanceArgs,
    ) -> Result<Result<MaintenancePreview, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.preview_queue_maintenance(args).await
    }

    /// Drain / clean / retry all failed.
    pub async fn run_queue_maintenance(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: QueueMaintenanceArgs,
        app: Option<crate::core::events::AppHandle>,
        attribution: super::actor::AuditAttribution,
    ) -> Result<Result<MaintenanceResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.run_queue_maintenance(args, app, attribution).await
    }

    /// Enqueue a job.
    pub async fn add_job(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: AddJobArgs,
        attribution: super::actor::AuditAttribution,
    ) -> Result<Result<AddJobResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.add_job(args, attribution).await
    }

    pub async fn retry_job(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: JobActionArgs,
        attribution: super::actor::AuditAttribution,
    ) -> Result<Result<JobActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.retry_job(args, attribution).await
    }

    /// Remove a job.
    pub async fn remove_job(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: JobActionArgs,
        attribution: super::actor::AuditAttribution,
    ) -> Result<Result<JobActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.remove_job(args, attribution).await
    }

    /// Promote a delayed job.
    pub async fn promote_job(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: JobActionArgs,
        attribution: super::actor::AuditAttribution,
    ) -> Result<Result<JobActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.promote_job(args, attribution).await
    }

    /// Bulk single-queue retry.
    pub async fn bulk_retry_jobs(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: BulkActionArgs,
        app: Option<&crate::core::events::AppHandle>,
    ) -> Result<Result<BulkActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.bulk_retry_jobs(args, app.cloned()).await
    }

    /// Bulk single-queue remove.
    pub async fn bulk_remove_jobs(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: BulkActionArgs,
        app: Option<&crate::core::events::AppHandle>,
    ) -> Result<Result<BulkActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.bulk_remove_jobs(args, app.cloned()).await
    }

    /// Bulk single-queue promote.
    pub async fn bulk_promote_jobs(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: BulkActionArgs,
        app: Option<&crate::core::events::AppHandle>,
    ) -> Result<Result<BulkActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.bulk_promote_jobs(args, app.cloned()).await
    }

    /// Pause the queue.
    pub async fn pause_queue(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: QueueActionArgs,
    ) -> Result<Result<QueueActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.pause_queue(args).await
    }

    /// Resume the queue.
    pub async fn resume_queue(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: QueueActionArgs,
    ) -> Result<Result<QueueActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.resume_queue(args).await
    }

    /// Pause a CRON / repeatable scheduler.
    pub async fn pause_scheduler(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: SchedulerActionArgs,
    ) -> Result<Result<SchedulerActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.pause_scheduler(args).await
    }

    /// Resume a paused scheduler.
    pub async fn resume_scheduler(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: SchedulerActionArgs,
    ) -> Result<Result<SchedulerActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.resume_scheduler(args).await
    }

    // ── agent-attributed variants (MCP path) ────────────────

    pub async fn bulk_retry_jobs_as(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: BulkActionArgs,
        attribution: super::actor::AuditAttribution,
    ) -> Result<Result<BulkActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.bulk_retry_jobs_as(args, None, attribution).await
    }

    pub async fn bulk_remove_jobs_as(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: BulkActionArgs,
        attribution: super::actor::AuditAttribution,
    ) -> Result<Result<BulkActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.bulk_remove_jobs_as(args, None, attribution).await
    }

    pub async fn bulk_promote_jobs_as(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: BulkActionArgs,
        attribution: super::actor::AuditAttribution,
    ) -> Result<Result<BulkActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.bulk_promote_jobs_as(args, None, attribution).await
    }

    pub async fn pause_queue_as(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: QueueActionArgs,
        attribution: super::actor::AuditAttribution,
    ) -> Result<Result<QueueActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.pause_queue_as(args, attribution).await
    }

    pub async fn resume_queue_as(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: QueueActionArgs,
        attribution: super::actor::AuditAttribution,
    ) -> Result<Result<QueueActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.resume_queue_as(args, attribution).await
    }

    /// Create or replace a job scheduler.
    pub async fn upsert_scheduler(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: UpsertSchedulerArgs,
        attribution: super::actor::AuditAttribution,
    ) -> Result<Result<SchedulerSaved, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.upsert_scheduler(args, attribution).await
    }

    /// Delete a job scheduler.
    pub async fn remove_scheduler(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: SchedulerActionArgs,
        attribution: super::actor::AuditAttribution,
    ) -> Result<Result<SchedulerActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.remove_scheduler(args, attribution).await
    }

    pub async fn pause_scheduler_as(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: SchedulerActionArgs,
        attribution: super::actor::AuditAttribution,
    ) -> Result<Result<SchedulerActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.pause_scheduler_as(args, attribution).await
    }

    pub async fn resume_scheduler_as(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
        args: SchedulerActionArgs,
        attribution: super::actor::AuditAttribution,
    ) -> Result<Result<SchedulerActionResult, RanchError>, ActorError> {
        let handle = self.get_or_spawn(connection_id, config).await;
        handle.resume_scheduler_as(args, attribution).await
    }

    /// Explicit shutdown for a specific connection. Called from
    /// `remove_connection` to drop the actor as soon as the SQLite
    /// row + keychain entries are gone. Returns once the actor's task
    /// has finished (bounded by a 1-second timeout per the cancellation
    /// contract; if the timeout fires we abandon the handle and let
    /// the runtime drop it).
    pub async fn shutdown(&self, connection_id: &str) {
        let entry = {
            let mut map = self.inner.lock().await;
            map.remove(connection_id)
        };
        if let Some(RegistryEntry { handle, join }) = entry {
            handle.signal_shutdown();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), join).await;
        }
    }

    /// Best-effort shutdown of every live actor. Called at app exit.
    pub async fn shutdown_all(&self) {
        let entries: Vec<RegistryEntry> = {
            let mut map = self.inner.lock().await;
            map.drain().map(|(_, e)| e).collect()
        };
        for entry in &entries {
            entry.handle.signal_shutdown();
        }
        for RegistryEntry { join, .. } in entries {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), join).await;
        }
    }

    /// `true` when no actors are currently live. Used by tests + future
    /// diagnostics (e.g. a debug command surfacing actor count).
    pub async fn is_empty(&self) -> bool {
        self.inner.lock().await.is_empty()
    }

    /// Number of currently-live actors. Used by tests + future
    /// diagnostics. O(1) lock-acquire + length query.
    pub async fn len(&self) -> usize {
        self.inner.lock().await.len()
    }

    async fn get_or_spawn(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
    ) -> ConnectionHandle {
        // Fast path: actor already spawned.
        {
            let map = self.inner.lock().await;
            if let Some(entry) = map.get(connection_id) {
                return entry.handle.clone();
            }
        }

        // Slow path: build the channel + token outside the lock so we
        // don't hold it across `tokio::spawn` (which would serialize
        // every spawn under the registry-wide mutex).
        let (tx, rx) = mpsc::channel::<ActorCommand>(64);
        let cancel = CancellationToken::new();
        let join = tokio::spawn(run_actor(rx, cancel.clone(), config.clone()));
        let new_handle = ConnectionHandle {
            tx,
            cancel,
            job_query_seq: Default::default(),
        };

        // Re-check + insert under the lock. If another caller raced and
        // inserted first, we discard our just-spawned actor.
        let mut map = self.inner.lock().await;
        if let Some(entry) = map.get(connection_id) {
            new_handle.signal_shutdown();
            return entry.handle.clone();
        }
        map.insert(
            connection_id.to_string(),
            RegistryEntry {
                handle: new_handle.clone(),
                join,
            },
        );
        new_handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::connections::TestConnectionError;
    use crate::core::connection_actor::ConnectionConfig;

    async fn unreachable_cfg() -> ConnectionConfig {
        // Port 1 is reserved/unused — connection refused fast on most
        // platforms, providing a deterministic "ping fails" target.
        //
        // `pool` is required for the destructive handlers
        // but the lifecycle / ping tests below never invoke them, so a
        // bare in-memory pool with no migrations is good enough.
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
        ConnectionConfig {
            host: "127.0.0.1".into(),
            port: 1,
            db: 0,
            username: None,
            password: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
            ssh_password: None,
            keychain: std::sync::Arc::new(crate::core::keychain::InMemoryKeychain::new()),
            initial_lock_state: crate::core::safety_lock::ReadOnlyState::Unlocked,
            connection_id: "test-conn".into(),
            connection_name: "Test".into(),
            env_color: Some("env-local".into()),
            pool,
        }
    }

    #[tokio::test]
    async fn first_ping_spawns_actor_and_returns_result() {
        let reg = ConnectionRegistry::new();
        let result = reg.ping("conn-x", &unreachable_cfg().await).await.unwrap();
        // The actor returned a structured result (failure shape, since the
        // host is unreachable) — the wiring is correct.
        assert!(!result.success);
        assert!(!reg.is_empty().await);
    }

    #[tokio::test]
    async fn second_ping_reuses_the_actor() {
        let reg = ConnectionRegistry::new();
        let _ = reg.ping("conn-x", &unreachable_cfg().await).await.unwrap();
        // Map should have exactly one entry.
        assert_eq!(reg.inner.lock().await.len(), 1);
        let _ = reg.ping("conn-x", &unreachable_cfg().await).await.unwrap();
        // Still exactly one — reused.
        assert_eq!(reg.inner.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn shutdown_removes_entry_and_finishes_task() {
        let reg = ConnectionRegistry::new();
        let _ = reg.ping("conn-x", &unreachable_cfg().await).await.unwrap();
        assert!(!reg.is_empty().await);

        let start = std::time::Instant::now();
        reg.shutdown("conn-x").await;
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 1_500, "shutdown took {elapsed:?}");
        assert!(reg.is_empty().await);
    }

    #[tokio::test]
    async fn shutdown_unknown_id_is_a_noop() {
        let reg = ConnectionRegistry::new();
        // Should return without panic and without doing anything.
        reg.shutdown("does-not-exist").await;
        assert!(reg.is_empty().await);
    }

    #[tokio::test]
    async fn shutdown_all_drains_every_actor() {
        let reg = ConnectionRegistry::new();
        for id in ["a", "b", "c"] {
            let _ = reg.ping(id, &unreachable_cfg().await).await.unwrap();
        }
        assert_eq!(reg.inner.lock().await.len(), 3);
        reg.shutdown_all().await;
        assert!(reg.is_empty().await);
    }

    #[tokio::test]
    async fn ssh_tunnel_attempts_real_open() {
        // The SSH path is live: the ping actually opens the russh session
        // (or fails fast when the key file is missing), instead of
        // short-circuiting with a placeholder error.
        use crate::core::ssh_config::{SshAuthMethod, SshTunnelConfig};
        let mut cfg = unreachable_cfg().await;
        cfg.ssh_tunnel = Some(SshTunnelConfig {
            host: "bastion.example.com".into(),
            port: 22,
            user: "ubuntu".into(),
            auth: SshAuthMethod::KeyFile {
                key_file_path: "/tmp/key-that-does-not-exist".into(),
            },
            jump_host: None,
        });
        let reg = ConnectionRegistry::new();
        let result = reg.ping("conn-ssh", &cfg).await.unwrap();
        // Key file missing → SshTunnel error variant (not a generic
        // placeholder error).
        match result.error.unwrap() {
            TestConnectionError::SshTunnel { reason } => {
                assert!(!reason.is_empty(), "the SSH error says why; got: {reason}");
            }
            other => panic!("expected SshTunnel, got {other:?}"),
        }
    }
}
