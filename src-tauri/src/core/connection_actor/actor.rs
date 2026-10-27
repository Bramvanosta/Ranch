//! `ConnectionActor` task body, command enum, and per-connection
//! runtime config.
//!
//! The actor itself is a `tokio::spawn`-able async function ([`run_actor`])
//! that listens on an `mpsc::Receiver<ActorCommand>` racing against a
//! [`tokio_util::sync::CancellationToken`]. Each `ActorCommand` variant
//! carries a `oneshot::Sender` for its reply.
//!
//! ## Cancellation contract
//!
//! Every Redis I/O the actor runs is wrapped in `tokio::select!` against
//! the cancellation token. On `Shutdown` the token fires; the actor's
//! loop drains pending messages with a sentinel "Cancelled" reply, then
//! exits. In-flight ops cancel within 1 second.
//!
//! ## Schema decoder integration
//!
//! The actor scaffold handles messages such as `Ping`, `ListQueues` and
//! `GetCounters`, and integrates `core::bullmq_schema` for decoding.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::handle::QueryTicket;
use crate::commands::connections::{PingConnectionResult, TestConnectionError};
use crate::commands::jobs::{
    AddJobArgs, AddJobResult, BulkActionArgs, BulkActionResult, BulkFailure, BulkProgressEvent,
    GetJobArgs, GetJobLogsArgs, JobActionArgs, JobActionResult, JobLogsOutput, ListFailedJobsArgs,
    ListFailureCausesArgs, ListJobsArgs, ListJobsOutput, ListRecentJobsArgs,
};
use crate::commands::queues::{
    CountQueueWorkersArgs, GetQueueMetricsArgs, GetThroughputArgs, ListCronJobsArgs,
    ListWorkersArgs, QueueActionArgs, QueueActionResult, QueueMaintenanceArgs, ReadQueueEventsArgs,
    SchedulerActionArgs, SchedulerActionResult, UpsertSchedulerArgs,
};
use crate::core::audit::{
    record as audit_record, AuditAction, AuditEntry, AuditOutcome, AuditSource, AuditTarget,
};
use crate::core::bullmq_schema::add_job;
use crate::core::bullmq_schema::flow::{self, FlowNode, FlowRoot, FlowRoots, FlowTree};
use crate::core::bullmq_schema::job_query::{self, JobQuery, JobRow};
use crate::core::bullmq_schema::maintenance::{self, BatchReply, MoveCode};
use crate::core::bullmq_schema::scheduler;
use crate::core::bullmq_schema::v5::V5Decoder;
use crate::core::bullmq_schema::v6::{V6Decoder, MARKER_TOKEN};
use crate::core::bullmq_schema::workers;
use crate::core::bullmq_schema::{
    decode_cron_hash, detect_schema, BullmqSchema, CleanState, CronJob, FailedJobsPage,
    FailureCauses, Job, JobDecoder, JobRef, JobState, KeyVariant, Keys, MaintenancePreview,
    MaintenanceResult, Queue, QueueCounters, QueueMaintenance, QueueMetrics, QueueRef,
    QueueWorkerCounts, QueueWorkers, RecentJobs, Schedule, SchedulerSaved, SchedulerSpec,
    StateRedisShape, V5Keys, V6Keys,
};
use crate::core::errors::RanchError;
use crate::core::keychain::KeychainAdapter;
use crate::core::mcp::gate::ApprovalGrant;
use crate::core::safety_lock::{check_lock_or_err, ReadOnlyState};
use crate::core::ssh_config::SshTunnelConfig;
use crate::core::ssh_tunnel::SshTunnel;
use crate::core::tls::{build_redis_client, connection_manager_config, validate_ca_pem};

/// Per-connection runtime config. Resolved at spawn time from
/// (SQLite row + keychain credential). Internal to the actor module —
/// not a wire type, not specta-derived.
///
/// **NOTE**: contains the SSH/Redis password in plaintext for the
/// actor's lifetime. This is acceptable per architecture (the trust
/// boundary is OS-level user isolation; the keychain protects only
/// at-rest credentials, not in-process). Password lifetime is bounded
/// by actor lifetime — `remove_connection` shuts the actor down,
/// dropping the config.
#[derive(Clone)]
pub struct ConnectionConfig {
    pub host: String,
    pub port: u16,
    pub db: u8,
    pub username: Option<String>,
    pub password: Option<String>,
    pub tls: bool,
    pub tls_custom_ca: Option<String>,
    pub ssh_tunnel: Option<SshTunnelConfig>,
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
    /// case where the row has no env — the audit row keeps a NULL.
    pub env_color: Option<String>,
    /// SQLite pool handle so destructive actor handlers
    /// (retry / remove / promote) can write audit entries via
    /// `core::audit::record`. `SqlitePool` is `Clone` + internally
    /// `Arc`-wrapped, so cloning is a cheap refcount bump.
    /// Read-only handlers (Ping, ListQueues, ListJobs, GetJob,
    /// GetJobLogs, ListCronJobs) ignore this field; they have no
    /// audit step.
    pub pool: sqlx::SqlitePool,
    /// SSH password fetched from the keychain at
    /// `resolve_connection_config` time. Only meaningful when
    /// `ssh_tunnel.auth.kind == "password"`. None otherwise. Same
    /// trust-boundary stance as `password`: plaintext in-process for
    /// the actor's lifetime, dropped on shutdown.
    pub ssh_password: Option<String>,
    /// Keychain handle for the SSH tunnel's TOFU host-key
    /// pin (`{id}::ssh::hostkey`). Cloning the Arc is a cheap refcount
    /// bump; the underlying adapter is `Send + Sync`.
    pub keychain: Arc<dyn KeychainAdapter>,
}

// Manual Debug impl — `dyn KeychainAdapter` doesn't impl Debug, so we
// skip that field. Useful only for tracing/log messages anyway.
impl std::fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("db", &self.db)
            .field("username", &self.username)
            .field("tls", &self.tls)
            .field("ssh_tunnel", &self.ssh_tunnel)
            .field("connection_id", &self.connection_id)
            .field("connection_name", &self.connection_name)
            .field("env_color", &self.env_color)
            .finish_non_exhaustive()
    }
}

/// Audit attribution carried alongside destructive actor
/// commands. UI callers pass `AuditAttribution::ui()`; MCP callers
/// populate `source = Agent` + the agent identifier from the MCP
/// `ClientInfo` (the initialize hook in `core::mcp`).
#[derive(Debug, Clone)]
pub struct AuditAttribution {
    pub source: AuditSource,
    pub agent_client: Option<String>,
    pub agent_transport: Option<String>,
    /// Set when a human approved this agent call in Ranch. The
    /// handler trades it (once) for the right to execute even on a
    /// session-locked connection; see `authorize_write`. Only
    /// `core::mcp::gate` can mint an `ApprovalGrant`.
    pub approval: Option<ApprovalGrant>,
}

impl AuditAttribution {
    /// UI-attributed action (the default for every existing Tauri
    /// command path). `agent_client` + `agent_transport` are None.
    pub fn ui() -> Self {
        Self {
            source: AuditSource::Ui,
            agent_client: None,
            agent_transport: None,
            approval: None,
        }
    }

    /// Agent-attributed action (MCP path). `agent_client` carries the
    /// MCP `ClientInfo.name` (e.g. `"claude-desktop"`); `agent_transport`
    /// is `"stdio"` for v1.
    pub fn agent(agent_client: impl Into<String>, agent_transport: impl Into<String>) -> Self {
        Self {
            source: AuditSource::Agent,
            agent_client: Some(agent_client.into()),
            agent_transport: Some(agent_transport.into()),
            approval: None,
        }
    }

    /// Attach the human approval obtained by the MCP gate.
    pub fn with_approval(mut self, grant: Option<ApprovalGrant>) -> Self {
        self.approval = grant;
        self
    }
}

/// Internal command type — what the registry sends across the mailbox.
/// Each variant carries a `oneshot::Sender` for its reply. Cancellation
/// is by the actor's `CancellationToken` firing.
pub(super) enum ActorCommand {
    Ping {
        reply_to: oneshot::Sender<PingConnectionResult>,
    },
    /// Discover queues + fetch counters in one go.
    ListQueues {
        reply_to: oneshot::Sender<Result<Vec<Queue>, RanchError>>,
    },
    /// Paginated job list for a (queue, state) pair.
    ListJobs {
        args: Box<ListJobsArgs>,
        /// Set for search / sort scans: lets the scan stop when
        /// a newer one has been requested.
        ticket: Option<QueryTicket>,
        reply_to: oneshot::Sender<Result<ListJobsOutput, RanchError>>,
    },
    /// Fetch + decode a single job by id (drill-down view).
    GetJob {
        args: Box<GetJobArgs>,
        reply_to: oneshot::Sender<Result<Job, RanchError>>,
    },
    /// Paginated read of BullMQ `Job.log()` entries from
    /// the `bull:{q}:{id}:logs` list.
    GetJobLogs {
        args: Box<GetJobLogsArgs>,
        reply_to: oneshot::Sender<Result<JobLogsOutput, RanchError>>,
    },
    /// List of CRON / repeatable schedulers for a queue.
    /// Reads `bull:{q}:repeat` (ZSET) + per-scheduler `bull:{q}:repeat:{key}`
    /// hashes via a non-atomic pipeline.
    ListCronJobs {
        args: Box<ListCronJobsArgs>,
        reply_to: oneshot::Sender<Result<Vec<CronJob>, RanchError>>,
    },
    /// 24h observability metrics for a queue: per-hour
    /// `completed` + `failed` counts (via `ZCOUNT` × 48), plus average
    /// processing time and queue delay over the last 100 completed
    /// jobs (via `ZRANGEBYSCORE` + `HMGET`).
    GetQueueMetrics {
        args: Box<GetQueueMetricsArgs>,
        reply_to: oneshot::Sender<Result<QueueMetrics, RanchError>>,
    },
    /// Live events of the connection's queues.
    ReadQueueEvents {
        args: Box<ReadQueueEventsArgs>,
        reply_to:
            oneshot::Sender<Result<crate::core::bullmq_schema::events::QueueEvents, RanchError>>,
    },
    /// The Redis server (version, latency, memory, ops/s).
    GetServerInfo {
        reply_to: oneshot::Sender<Result<crate::core::redis_info::ServerInfo, RanchError>>,
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
        reply_to:
            oneshot::Sender<Result<crate::core::bullmq_schema::topology::Topology, RanchError>>,
    },
    /// The summary of a Timeline window.
    SummarizeWindow {
        args: Box<crate::commands::queues::SummarizeWindowArgs>,
        reply_to: oneshot::Sender<
            Result<crate::core::bullmq_schema::timeline::WindowSummary, RanchError>,
        >,
    },
    /// Throughput of every queue, native metrics only.
    GetThroughput {
        args: Box<GetThroughputArgs>,
        reply_to: oneshot::Sender<Result<crate::core::bullmq_schema::Throughput, RanchError>>,
    },
    /// BullMQ workers attached to a queue, from `CLIENT LIST`
    /// (see `bullmq_schema::workers`). A refusal is cached per connection.
    ListWorkers {
        args: Box<ListWorkersArgs>,
        reply_to: oneshot::Sender<Result<QueueWorkers, RanchError>>,
    },
    /// Worker counts of many queues from one `CLIENT LIST`.
    CountQueueWorkers {
        args: Box<CountQueueWorkersArgs>,
        reply_to: oneshot::Sender<Result<QueueWorkerCounts, RanchError>>,
    },
    /// Failed jobs across every queue of the connection,
    /// newest first (bounded scan; see `bullmq_schema::failed`).
    ListFailedJobs {
        args: Box<ListFailedJobsArgs>,
        reply_to: oneshot::Sender<Result<FailedJobsPage, RanchError>>,
    },
    /// Failed jobs grouped by cause.
    ListFailureCauses {
        args: Box<ListFailureCausesArgs>,
        reply_to: oneshot::Sender<Result<FailureCauses, RanchError>>,
    },
    /// The jobs that ran last — one queue, or the whole connection merged
    /// (bounded; see `bullmq_schema::recent`).
    ListRecentJobs {
        args: Box<ListRecentJobsArgs>,
        reply_to: oneshot::Sender<Result<RecentJobs, RanchError>>,
    },
    /// Read the current in-memory lock state. No Redis IO.
    /// `unlock_connection` / `lock_connection` commands mutate via
    /// `Unlock` / `Lock` below; this is a pure read so the UI can
    /// hydrate the `ProductionLockBanner` on mount.
    GetLockState {
        reply_to: oneshot::Sender<ReadOnlyState>,
    },
    /// Flip the actor's `lock_state` to `Unlocked` and
    /// append an `unlockConnection` audit entry. Returns the new
    /// state on success; returns the audit-write error otherwise.
    Unlock {
        reply_to: oneshot::Sender<Result<ReadOnlyState, RanchError>>,
    },
    /// Flip the actor's `lock_state` to `Locked` and
    /// append a `lockConnection` audit entry. Symmetric to `Unlock`.
    Lock {
        reply_to: oneshot::Sender<Result<ReadOnlyState, RanchError>>,
    },
    /// The flow (parent / child tree) a job belongs to.
    GetFlow {
        args: Box<GetJobArgs>,
        reply_to: oneshot::Sender<Result<FlowTree, RanchError>>,
    },
    /// Flow roots found in a queue (bounded discovery).
    ListFlowRoots {
        args: Box<QueueActionArgs>,
        reply_to: oneshot::Sender<Result<FlowRoots, RanchError>>,
    },
    /// How many jobs a maintenance operation would touch. Read-only.
    PreviewQueueMaintenance {
        args: Box<QueueMaintenanceArgs>,
        reply_to: oneshot::Sender<Result<MaintenancePreview, RanchError>>,
    },
    /// Drain / clean / retry all failed: batched Lua scripts
    /// (`bullmq_schema::maintenance`), progress events, one audit row.
    RunQueueMaintenance {
        args: Box<QueueMaintenanceArgs>,
        attribution: AuditAttribution,
        app: Option<crate::core::events::AppHandle>,
        reply_to: oneshot::Sender<Result<MaintenanceResult, RanchError>>,
    },
    /// Enqueue a job: one atomic Lua script (see
    /// `bullmq_schema::add_job`). Gated + audited like every other write.
    AddJob {
        args: Box<AddJobArgs>,
        attribution: AuditAttribution,
        reply_to: oneshot::Sender<Result<AddJobResult, RanchError>>,
    },
    /// single-job destructive: retry a failed job back
    /// into the waiting list. Gated by `check_lock_or_err`; audited
    /// via `core::audit::record` (success OR failure). It
    /// carries `AuditAttribution` so MCP callers can record
    /// `source = Agent` + the agent client identifier.
    RetryJob {
        args: Box<JobActionArgs>,
        attribution: AuditAttribution,
        reply_to: oneshot::Sender<Result<JobActionResult, RanchError>>,
    },
    /// single-job destructive: remove a job from every
    /// state container + the job hash + logs.
    RemoveJob {
        args: Box<JobActionArgs>,
        attribution: AuditAttribution,
        reply_to: oneshot::Sender<Result<JobActionResult, RanchError>>,
    },
    /// single-job destructive: promote a delayed job
    /// into the waiting list.
    PromoteJob {
        args: Box<JobActionArgs>,
        attribution: AuditAttribution,
        reply_to: oneshot::Sender<Result<JobActionResult, RanchError>>,
    },
    /// Bulk single-queue retry. ONE lock check at the
    /// top, per-job MULTI/EXEC inside a loop, ONE consolidated audit
    /// row at the end. Progress events emitted via the optional
    /// `app` handle every `BULK_PROGRESS_CHUNK` jobs.
    BulkRetryJobs {
        args: Box<BulkActionArgs>,
        attribution: AuditAttribution,
        app: Option<crate::core::events::AppHandle>,
        reply_to: oneshot::Sender<Result<BulkActionResult, RanchError>>,
    },
    /// Bulk single-queue remove. Same shape as
    /// `BulkRetryJobs`.
    BulkRemoveJobs {
        args: Box<BulkActionArgs>,
        attribution: AuditAttribution,
        app: Option<crate::core::events::AppHandle>,
        reply_to: oneshot::Sender<Result<BulkActionResult, RanchError>>,
    },
    /// Bulk single-queue promote. Same shape as
    /// `BulkRetryJobs`.
    BulkPromoteJobs {
        args: Box<BulkActionArgs>,
        attribution: AuditAttribution,
        app: Option<crate::core::events::AppHandle>,
        reply_to: oneshot::Sender<Result<BulkActionResult, RanchError>>,
    },
    /// Pause the queue's `:wait` list (rename to `:paused`)
    /// + flip the `:meta` `paused` flag. Idempotent.
    PauseQueue {
        args: Box<QueueActionArgs>,
        attribution: AuditAttribution,
        reply_to: oneshot::Sender<Result<QueueActionResult, RanchError>>,
    },
    /// Symmetric to `PauseQueue` — rename `:paused` back
    /// to `:wait` + clear the `:meta` flag.
    ResumeQueue {
        args: Box<QueueActionArgs>,
        attribution: AuditAttribution,
        reply_to: oneshot::Sender<Result<QueueActionResult, RanchError>>,
    },
    /// Pause a CRON / repeatable scheduler by removing
    /// its entry from the `:repeat` ZSET + flagging the per-scheduler
    /// hash. Ranch-original (BullMQ has no public pause API for
    /// repeatables).
    PauseScheduler {
        args: Box<SchedulerActionArgs>,
        attribution: AuditAttribution,
        reply_to: oneshot::Sender<Result<SchedulerActionResult, RanchError>>,
    },
    /// Create or replace a job scheduler (`upsert_scheduler.lua`).
    UpsertScheduler {
        args: Box<UpsertSchedulerArgs>,
        attribution: AuditAttribution,
        reply_to: oneshot::Sender<Result<SchedulerSaved, RanchError>>,
    },
    /// Delete a job scheduler and its pending job.
    RemoveScheduler {
        args: Box<SchedulerActionArgs>,
        attribution: AuditAttribution,
        reply_to: oneshot::Sender<Result<SchedulerActionResult, RanchError>>,
    },
    /// re-arm a paused scheduler with the current
    /// timestamp as the next-run score.
    ResumeScheduler {
        args: Box<SchedulerActionArgs>,
        attribution: AuditAttribution,
        reply_to: oneshot::Sender<Result<SchedulerActionResult, RanchError>>,
    },
    Shutdown,
}

/// The actor's one Redis connection: multiplexed (concurrent
/// requests share it), reconnects by itself, cheap to clone into a handler.
type RedisConn = redis::aio::ConnectionManager;

/// The persistent connection and the tunnel address it was
/// opened through (`None` = direct), so a re-opened tunnel gets a new one.
struct CachedRedis {
    conn: RedisConn,
    via: Option<std::net::SocketAddr>,
}

/// Emit a `bulk:progress` event every N completed jobs
/// AND on the last job. Chunk size of 10 keeps the progress bar
/// smooth without flooding the event bus (247 events → 25 events).
const BULK_PROGRESS_CHUNK: usize = 10;

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
struct ActorState {
    cfg: ConnectionConfig,
    /// Cached BullMQ schema. None until first successful detection.
    /// Re-attempted on each `ListQueues` call until cached, then
    /// frozen for the actor's lifetime — see module docs for the
    /// invalidation rationale (v1: re-add the connection to refresh).
    cached_schema: Arc<std::sync::Mutex<Option<BullmqSchema>>>,
    /// read-only lock state. Initialized from
    /// `cfg.initial_lock_state` at spawn (which the registry derived
    /// from the `connections.is_locked` column). Mutated only via
    /// `ActorCommand::Unlock` / `ActorCommand::Lock`. In-memory only —
    /// restart re-locks.
    lock_state: ReadOnlyState,
    /// Cached SSH tunnel handle. `None` for non-SSH
    /// connections AND when the tunnel has not yet been opened. The
    /// first Redis op lazily establishes the tunnel + stores it here;
    /// subsequent ops reuse. Invalidated to `None` on Redis-side
    /// failure (the failure may be on the SSH side; next call
    /// re-tunnels). Cleanup is implicit: drop on actor shutdown fires
    /// the tunnel's Drop impl, which closes the SSH session.
    /// With the persistent connection, in [`Link`]: shared by
    /// concurrent reads, and locked while one of them opens it.
    link: Arc<tokio::sync::Mutex<Link>>,
    /// Capability probe result for `CLIENT LIST`. `Some(reason)`
    /// once this Redis has refused it (managed providers, ACLs): later calls
    /// answer "unsupported" without asking again. `None` = not refused so
    /// far. Lives as long as the actor — editing the connection respawns it.
    client_list_denied: Arc<std::sync::Mutex<Option<String>>>,
    /// The last `CLIENT LIST` reply and when it arrived, locked
    /// while one is read: the Workers tab asks once per queue at the same
    /// moment, and they all share one read.
    client_list_cache: Arc<tokio::sync::Mutex<Option<(std::time::Instant, String)>>>,
    /// The queues found by the last SCAN, shared with the task
    /// that refreshes them in the background.
    queue_registry: Arc<std::sync::Mutex<RegistrySlot>>,
}

/// The actor's way to Redis: the SSH tunnel and the persistent
/// connection (before it was kept, each command paid a TCP, TLS and
/// SSH-channel handshake). Both opened on first use.
#[derive(Default)]
struct Link {
    ssh_tunnel: Option<Arc<SshTunnel>>,
    redis: Option<CachedRedis>,
}

/// Entries read per XREAD, all streams together (capped).
const FOLLOW_COUNT: usize = 500;
/// How long one XREAD waits for news before asking again.
const FOLLOW_BLOCK_MS: usize = 5_000;
/// At most one `queue:events` per this: a busy Redis writes
/// events all the time, the app refreshes in batches.
const FOLLOW_MIN_GAP: std::time::Duration = std::time::Duration::from_millis(300);

/// A connection of its own for the blocking reads (a blocked
/// XREAD would hold every request queued behind it on the shared one), with
/// no reply timeout: the server answers when it has news or after
/// `FOLLOW_BLOCK_MS`.
async fn open_follower_connection(
    state: &ActorState,
) -> Result<redis::aio::MultiplexedConnection, RanchError> {
    let validated_ca = validate_ca_pem(state.cfg.tls_custom_ca.as_deref())?;
    let (url, ca) = {
        let mut link = state.link.lock().await;
        if state.cfg.ssh_tunnel.is_some() {
            let local = ensure_ssh_tunnel(&state.cfg, &mut link).await?;
            (build_redis_url_via_local(&state.cfg, local), None)
        } else {
            (build_redis_url_from_config(&state.cfg), validated_ca)
        }
    };
    let client = build_redis_client(&url, ca.as_deref()).map_err(redis_to_ranch_err)?;
    let config = redis::AsyncConnectionConfig::new()
        .set_response_timeout(None)
        .set_connection_timeout(Some(crate::core::tls::REDIS_CONNECT_TIMEOUT));
    client
        .get_multiplexed_async_connection_with_config(&config)
        .await
        .map_err(redis_to_ranch_err)
}

type StreamRead = Option<Vec<(String, Vec<(String, Vec<String>)>)>>;

/// Where the follower sends what it read: the app's windows
/// (`queue:events`), or a test.
pub type EventSink = Arc<dyn Fn(crate::commands::queues::QueueEventsNotice) + Send + Sync>;

/// Read the queues' event streams (`XREAD BLOCK`) — BullMQ
/// writes them for its own `QueueEvents`, so no Redis setting is needed —
/// and tell the windows which queues moved (`queue:events`), so they refresh
/// those instead of polling everything. Runs while a window follows the
/// connection (the visible one, while shown); the queues are the
/// registry's, new ones join on the next read.
async fn follow_events(
    state: ActorState,
    sink: EventSink,
    token: CancellationToken,
    mut ready: Option<oneshot::Sender<bool>>,
) {
    use crate::commands::queues::{QueueEventsNotice, QueueEventsSeen};
    let pause = |ms: u64| tokio::time::sleep(std::time::Duration::from_millis(ms));
    let mut conn: Option<redis::aio::MultiplexedConnection> = None;
    // Where each stream was read up to. A stream read for the first time
    // starts at the Redis clock when following began (not `$`, which would
    // drop what arrives between two reads).
    let mut cursors: HashMap<String, String> = HashMap::new();
    let mut start: Option<(i64, std::time::Instant)> = None;
    let mut last_emit = std::time::Instant::now() - FOLLOW_MIN_GAP;
    loop {
        if token.is_cancelled() {
            return;
        }
        if conn.is_none() {
            match open_follower_connection(&state).await {
                Ok(c) => conn = Some(c),
                Err(e) => {
                    tracing::warn!(error = %e, "event follower could not connect; retrying");
                    if let Some(ready) = ready.take() {
                        let _ = ready.send(true);
                    }
                    tokio::select! { _ = pause(2_000) => continue, _ = token.cancelled() => return }
                }
            }
        }
        let Some(c) = conn.as_mut() else { continue };
        if start.is_none() {
            let time: Result<(String, String), _> = redis::cmd("TIME").query_async(c).await;
            let server_ms = time
                .ok()
                .and_then(|(s, us)| {
                    Some(s.parse::<i64>().ok()? * 1_000 + us.parse::<i64>().ok()? / 1_000)
                })
                .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
            start = Some((server_ms, std::time::Instant::now()));
            if let Some(ready) = ready.take() {
                let _ = ready.send(true);
            }
        }
        let queues: Vec<QueueRef> = lock(&state.queue_registry)
            .current
            .as_ref()
            .map(|r| r.queues.iter().map(|(q, _)| q.clone()).collect())
            .unwrap_or_default();
        if queues.is_empty() {
            tokio::select! { _ = pause(1_000) => continue, _ = token.cancelled() => return }
        }
        let (start_ms, started) = start.unwrap_or((0, std::time::Instant::now()));
        let keys: Vec<(String, QueueRef)> =
            queues.into_iter().map(|q| (V5Keys.events(&q), q)).collect();
        let mut cmd = redis::cmd("XREAD");
        cmd.arg("COUNT")
            .arg(FOLLOW_COUNT)
            .arg("BLOCK")
            .arg(FOLLOW_BLOCK_MS)
            .arg("STREAMS");
        for (key, _) in &keys {
            cmd.arg(key);
        }
        for (key, _) in &keys {
            match cursors.get(key) {
                Some(id) => cmd.arg(id),
                None => cmd.arg(format!(
                    "{}-0",
                    start_ms + started.elapsed().as_millis() as i64
                )),
            };
        }
        let read: Result<StreamRead, redis::RedisError> = tokio::select! {
            r = cmd.query_async(c) => r,
            _ = token.cancelled() => return,
        };
        let streams = match read {
            Ok(Some(streams)) => streams,
            Ok(None) => continue, // nothing new within the block
            Err(e) => {
                tracing::warn!(error = %e, "event follower read failed; reconnecting");
                conn = None;
                tokio::select! { _ = pause(2_000) => continue, _ = token.cancelled() => return }
            }
        };
        let mut seen: Vec<QueueEventsSeen> = Vec::new();
        for (key, entries) in streams {
            let Some((last_id, _)) = entries.last() else {
                continue;
            };
            cursors.insert(key.clone(), last_id.clone());
            let Some((_, queue_ref)) = keys.iter().find(|(k, _)| *k == key) else {
                continue;
            };
            let mut events: Vec<String> = Vec::new();
            for (_, fields) in &entries {
                let name = fields
                    .chunks(2)
                    .find(|kv| kv.first().is_some_and(|k| k == "event"))
                    .and_then(|kv| kv.get(1).cloned());
                if let Some(name) = name {
                    if !events.contains(&name) {
                        events.push(name);
                    }
                }
            }
            seen.push(QueueEventsSeen {
                queue_ref: queue_ref.clone(),
                events,
                count: entries.len() as u32,
            });
        }
        if seen.is_empty() {
            continue;
        }
        // Batch: at most one notice per FOLLOW_MIN_GAP.
        let since = last_emit.elapsed();
        if since < FOLLOW_MIN_GAP {
            tokio::select! { _ = tokio::time::sleep(FOLLOW_MIN_GAP - since) => {}, _ = token.cancelled() => return }
        }
        sink(QueueEventsNotice {
            connection_id: state.cfg.connection_id.clone(),
            queues: seen,
        });
        last_emit = std::time::Instant::now();
    }
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

/// One read per item, all sent at once on the multiplexed
/// connection: the round trips overlap, so N queues cost about one. Results
/// come back in the items' order; `None` where a task panicked.
async fn read_each<I, T, F, Fut>(
    conn: &RedisConn,
    items: Vec<I>,
    token: &CancellationToken,
    read: F,
) -> Result<Vec<Option<T>>, RanchError>
where
    F: Fn(RedisConn, I) -> Fut,
    Fut: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let mut out: Vec<Option<T>> = (0..items.len()).map(|_| None).collect();
    let mut set = tokio::task::JoinSet::new();
    for (i, item) in items.into_iter().enumerate() {
        let fut = read(conn.clone(), item);
        set.spawn(async move { (i, fut.await) });
    }
    loop {
        tokio::select! {
            next = set.join_next() => match next {
                None => break,
                Some(Ok((i, value))) => out[i] = Some(value),
                Some(Err(e)) => tracing::error!(error = %e, "read task panicked"),
            },
            _ = token.cancelled() => {
                return Err(RanchError::InvalidInput {
                    reason: "Operation cancelled".to_string(),
                });
            }
        }
    }
    Ok(out)
}

/// A [`read_each`] item whose task panicked, as a read error.
fn read_panicked() -> redis::RedisError {
    redis::RedisError::from((redis::ErrorKind::Client, "read task panicked"))
}

/// A shared cache's value; a panic elsewhere never makes it unusable.
fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// How old the queue registry may get before a refresh starts.
/// It is served while it refreshes, so a new queue shows up within ~30 s.
const REGISTRY_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// The BullMQ queues of a Redis (meta key with a version) and
/// their schema, as the last SCAN found them.
#[derive(Clone)]
struct QueueRegistry {
    queues: Vec<(QueueRef, BullmqSchema)>,
    scanned_at: std::time::Instant,
}

#[derive(Default)]
struct RegistrySlot {
    current: Option<QueueRegistry>,
    refreshing: bool,
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
                    ActorCommand::ListCronJobs { args, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_list_cron_jobs(&mut st, &args, &tk).await
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
                    ActorCommand::GetThroughput { args, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_get_throughput(&mut st, &args, &tk).await
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
                    ActorCommand::ListWorkers { args, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_list_workers(&mut st, &args, &tk).await
                        });
                    }
                    ActorCommand::CountQueueWorkers { args, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_count_queue_workers(&mut st, &args, &tk).await
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
                    ActorCommand::ListFailureCauses { args, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_list_failure_causes(&mut st, &args, &tk).await
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
                        // after the actor returns — that's the only
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
                    ActorCommand::ListFlowRoots { args, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_list_flow_roots(&mut st, &args, &tk).await
                        });
                    }
                    ActorCommand::PreviewQueueMaintenance { args, reply_to } => {
                        let mut st = state.clone();
                        let tk = token.clone();
                        let permits = Arc::clone(&reads);
                        spawn_read(reply_to, async move {
                            let _permit = permits.acquire_owned().await;
                            handle_preview_queue_maintenance(&mut st, &args, &tk).await
                        });
                    }
                    ActorCommand::RunQueueMaintenance { args, attribution, app, reply_to } => {
                        let result = handle_run_queue_maintenance(&mut state, &args, &attribution, app.as_ref(), &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::AddJob { args, attribution, reply_to } => {
                        let result = handle_add_job(&mut state, &args, &attribution, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::RetryJob { args, attribution, reply_to } => {
                        let result = handle_retry_job(&mut state, &args, &attribution, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::RemoveJob { args, attribution, reply_to } => {
                        let result = handle_remove_job(&mut state, &args, &attribution, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::PromoteJob { args, attribution, reply_to } => {
                        let result = handle_promote_job(&mut state, &args, &attribution, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::BulkRetryJobs { args, attribution, app, reply_to } => {
                        let result = handle_bulk_retry_jobs(&mut state, &args, &attribution, app.as_ref(), &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::BulkRemoveJobs { args, attribution, app, reply_to } => {
                        let result = handle_bulk_remove_jobs(&mut state, &args, &attribution, app.as_ref(), &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::BulkPromoteJobs { args, attribution, app, reply_to } => {
                        let result = handle_bulk_promote_jobs(&mut state, &args, &attribution, app.as_ref(), &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::PauseQueue { args, attribution, reply_to } => {
                        let result = handle_pause_queue(&mut state, &args, &attribution, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::ResumeQueue { args, attribution, reply_to } => {
                        let result = handle_resume_queue(&mut state, &args, &attribution, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::UpsertScheduler { args, attribution, reply_to } => {
                        let result = handle_upsert_scheduler(&mut state, &args, &attribution, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::RemoveScheduler { args, attribution, reply_to } => {
                        let result = handle_remove_scheduler(&mut state, &args, &attribution, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::PauseScheduler { args, attribution, reply_to } => {
                        let result = handle_pause_scheduler(&mut state, &args, &attribution, &token).await;
                        let _ = reply_to.send(result);
                    }
                    ActorCommand::ResumeScheduler { args, attribution, reply_to } => {
                        let result = handle_resume_scheduler(&mut state, &args, &attribution, &token).await;
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

/// Handle a `Ping` message. Mirrors the semantics of
/// `commands::connections::ping_with_args` but races the redis op against the actor's cancellation token.
///
/// Pings over the actor's persistent connection (through the
/// SSH tunnel when there is one), opening it if needed.
async fn handle_ping(state: &mut ActorState, token: &CancellationToken) -> PingConnectionResult {
    // Validate CA PEM up-front. Bad PEM → typed TLS error.
    if let Err(e) = validate_ca_pem(state.cfg.tls_custom_ca.as_deref()) {
        return PingConnectionResult {
            success: false,
            latency_ms: None,
            error: Some(TestConnectionError::TlsHandshake {
                reason: e.to_string(),
            }),
        };
    }

    // The ping goes through the persistent connection (opened
    // now if needed, bounded by its connect timeout); the latency is the
    // PING round trip alone, not a handshake.
    let open = tokio::time::timeout(PING_OPEN_TIMEOUT, open_redis_for_actor(state, token));
    let mut conn = match open.await {
        Ok(Ok(conn)) => conn,
        Ok(Err(RanchError::Redis(e))) => {
            return PingConnectionResult {
                success: false,
                latency_ms: None,
                error: Some(map_redis_err(e)),
            };
        }
        Ok(Err(e)) => {
            return PingConnectionResult {
                success: false,
                latency_ms: None,
                error: Some(ranch_err_to_test_err(&e)),
            };
        }
        Err(_) => {
            return PingConnectionResult {
                success: false,
                latency_ms: None,
                error: Some(TestConnectionError::Timeout),
            };
        }
    };

    let ping_cmd = redis::cmd("PING");
    let start = std::time::Instant::now();
    let ping = tokio::time::timeout(
        PING_REPLY_TIMEOUT,
        ping_cmd.query_async::<String>(&mut conn),
    );
    let result = tokio::select! {
        r = ping => r,
        _ = token.cancelled() => return actor_cancelled_result(),
    };
    let latency_ms = start.elapsed().as_millis() as u32;

    match result {
        Ok(Ok(_)) => PingConnectionResult {
            success: true,
            latency_ms: Some(latency_ms),
            error: None,
        },
        Ok(Err(e)) => PingConnectionResult {
            success: false,
            latency_ms: None,
            error: Some(map_redis_err(e)),
        },
        Err(_) => PingConnectionResult {
            success: false,
            latency_ms: None,
            error: Some(TestConnectionError::Timeout),
        },
    }
}

/// Opening the connection for a ping: the connect timeout plus a
/// cold SSH handshake. Was 2 s direct / 8 s SSH, too short for a slow link.
const PING_OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
/// One PING on an open connection: a link of up to ~1 s round
/// trip is slow, not unreachable.
const PING_REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Collapse the three SSH RanchError variants into the
/// existing TestConnectionError surface. Used by `handle_ping` to
/// preserve the wire shape (PingConnectionResult.error type).
fn ranch_err_to_test_err(e: &RanchError) -> TestConnectionError {
    match e {
        RanchError::SshAuthFailed { reason } => TestConnectionError::SshTunnel {
            reason: reason.clone(),
        },
        RanchError::SshKeyFileError { path, reason } => TestConnectionError::SshTunnel {
            reason: format!("Key file at {path}: {reason}"),
        },
        RanchError::SshTunnelFailed { reason } => TestConnectionError::SshTunnel {
            reason: reason.clone(),
        },
        other => TestConnectionError::Other {
            reason: other.to_string(),
        },
    }
}

/// get-or-open the cached SSH tunnel. Returns the local
/// loopback address ready for the Redis URL builder.
async fn ensure_ssh_tunnel(
    cfg: &ConnectionConfig,
    link: &mut Link,
) -> Result<std::net::SocketAddr, RanchError> {
    if let Some(tunnel) = link.ssh_tunnel.as_ref() {
        return Ok(tunnel.local_addr());
    }
    let ssh_cfg = cfg
        .ssh_tunnel
        .as_ref()
        .ok_or_else(|| RanchError::SshTunnelFailed {
            reason: "ensure_ssh_tunnel called on a non-SSH connection".to_string(),
        })?;
    let opened = crate::core::ssh_tunnel::open(
        ssh_cfg,
        cfg.ssh_password.as_deref(),
        (&cfg.host, cfg.port),
        cfg.keychain.clone(),
        &cfg.connection_id,
    )
    .await?;
    let local = opened.tunnel.local_addr();
    link.ssh_tunnel = Some(Arc::new(opened.tunnel));
    Ok(local)
}

/// Build the Redis URL targeting the SSH-tunnel's local
/// loopback listener. The SSH session is the encryption layer, so the
/// URL scheme is always `redis://` (NOT `rediss://`) regardless of
/// `cfg.tls`. Username, password, db are preserved.
fn build_redis_url_via_local(cfg: &ConnectionConfig, local: std::net::SocketAddr) -> String {
    let bare = format!("redis://{}:{}/{}", local.ip(), local.port(), cfg.db);
    let mut u = match Url::parse(&bare) {
        Ok(u) => u,
        Err(_) => return bare,
    };
    if let Some(user) = cfg.username.as_deref() {
        let _ = u.set_username(user);
    }
    if let Some(pass) = cfg.password.as_deref() {
        let _ = u.set_password(Some(pass));
    }
    u.to_string()
}

/// Sentinel reply when the actor is shutting down. Maps cleanly to the
/// existing `TestConnectionError::Other` variant so wire shapes stay
/// stable.
pub(super) fn actor_cancelled_result() -> PingConnectionResult {
    PingConnectionResult {
        success: false,
        latency_ms: None,
        error: Some(TestConnectionError::Other {
            reason: "Connection actor shut down".to_string(),
        }),
    }
}

/// Build the Redis URL from `ConnectionConfig`. Mirrors
/// `commands::connections::build_redis_url` (which takes a
/// `TestConnectionArgs`); we duplicate ~10 lines here rather than
/// share that helper because the input shapes are different; they could be
/// consolidated if more callers need it.
fn build_redis_url_from_config(cfg: &ConnectionConfig) -> String {
    let scheme = if cfg.tls { "rediss" } else { "redis" };
    let bare = format!(
        "{scheme}://{host}:{port}/{db}",
        host = cfg.host,
        port = cfg.port,
        db = cfg.db
    );
    let mut u = match Url::parse(&bare) {
        Ok(u) => u,
        Err(_) => return bare,
    };
    if let Some(user) = cfg.username.as_deref() {
        let _ = u.set_username(user);
    }
    if let Some(pass) = cfg.password.as_deref() {
        let _ = u.set_password(Some(pass));
    }
    u.to_string()
}

/// TLS-detection list mirrored from `commands::connections::map_redis_err`
///. Kept locally so the actor doesn't depend on a
/// `pub(crate)` symbol from a sibling module just for this — same
/// trade-off as the URL-building duplication above.
const TLS_NEEDLES: &[&str] = &[
    "tls",
    "certificate",
    "handshake",
    "verify",
    "unknown ca",
    "self signed",
    "self-signed",
    "expired",
];

fn map_redis_err(e: redis::RedisError) -> TestConnectionError {
    use redis::ErrorKind;
    let kind = e.kind();
    let msg = e.to_string();
    let lower = msg.to_lowercase();
    if TLS_NEEDLES.iter().any(|n| lower.contains(n)) {
        return TestConnectionError::TlsHandshake { reason: msg };
    }
    match kind {
        ErrorKind::AuthenticationFailed => TestConnectionError::AuthFailed,
        ErrorKind::Io => TestConnectionError::CannotReach { reason: msg },
        _ => TestConnectionError::Other { reason: msg },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// handle_list_queues + helpers
// ─────────────────────────────────────────────────────────────────────────────

/// List the queues and their counters.
///
/// The queues come from the actor's registry (a SCAN, cached
/// and refreshed in the background, see [`queue_registry`]); every queue's
/// counters and pause flag are then read in **one pipeline**: one round trip
/// whatever the number of queues. A failed read is an error, never
/// zeros.
///
/// Returns `Vec<Queue>` sorted by `queue_ref.name` (case-insensitive).
/// Empty list when no queues are found. Schema-not-recognized when the
/// only discovered queues use unsupported BullMQ versions.
async fn handle_list_queues(
    state: &mut ActorState,
    token: &CancellationToken,
) -> Result<Vec<Queue>, RanchError> {
    let mut conn = open_redis_for_actor(state, token).await?;
    let detected = queue_registry(state, &conn, token).await?;
    if detected.is_empty() {
        return Ok(Vec::new());
    }

    // Cache the schema (first-success-wins). If every detected schema is
    // Unknown, surface a typed schema-not-recognized error.
    let schema = pick_or_cache_schema(&mut lock(&state.cached_schema), &detected)?;
    let keys: KeyVariant = match schema {
        BullmqSchema::V5 => KeyVariant::V5(V5Keys),
        BullmqSchema::V6 => KeyVariant::V6(V6Keys),
        // pick_or_cache_schema returns Err for Unknown.
        BullmqSchema::Unknown { .. } => unreachable!("pick_or_cache_schema rejects Unknown"),
    };

    let mut pipe = redis::pipe();
    for (qref, _) in &detected {
        push_counter_reads(&mut pipe, keys, qref);
    }
    let replies: Vec<redis::Value> = tokio::select! {
        r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::InvalidInput {
                reason: "Operation cancelled".to_string(),
            });
        }
    };

    let mut queues: Vec<Queue> = Vec::with_capacity(detected.len());
    let mut gone = false;
    for ((qref, _), reply) in detected.iter().zip(replies.chunks(COUNTER_READS)) {
        match counters_from(qref, reply).map_err(redis_to_ranch_err)? {
            Some(queue) => queues.push(queue),
            // Deleted since the SCAN (obliterated): left out, and the
            // registry is refreshed on the next call.
            None => gone = true,
        }
    }
    if gone {
        if let Ok(mut slot) = state.queue_registry.lock() {
            if let Some(current) = slot.current.as_mut() {
                current.scanned_at -= REGISTRY_TTL;
            }
        }
    }

    // Sort by queue name (case-insensitive) for stable polling output.
    queues.sort_by(|a, b| {
        a.queue_ref
            .name
            .to_lowercase()
            .cmp(&b.queue_ref.name.to_lowercase())
    });

    Ok(queues)
}

/// The queues of this Redis. The first call scans; later calls
/// answer from the cache at once, and once it is older than
/// [`REGISTRY_TTL`] a background task rescans (one at a time) while the
/// cached list keeps being served.
async fn queue_registry(
    state: &mut ActorState,
    conn: &RedisConn,
    token: &CancellationToken,
) -> Result<Vec<(QueueRef, BullmqSchema)>, RanchError> {
    let slot = Arc::clone(&state.queue_registry);
    let (cached, refresh) = {
        let mut guard = slot.lock().map_err(|_| RanchError::InvalidInput {
            reason: "queue registry poisoned".to_string(),
        })?;
        let cached = guard.current.clone();
        let stale = cached
            .as_ref()
            .is_some_and(|r| r.scanned_at.elapsed() >= REGISTRY_TTL);
        let refresh = stale && !guard.refreshing;
        if refresh {
            guard.refreshing = true;
        }
        (cached, refresh)
    };

    let Some(cached) = cached else {
        let mut conn = conn.clone();
        let queues = discover_queues(&mut conn, token).await?;
        if let Ok(mut guard) = slot.lock() {
            guard.current = Some(QueueRegistry {
                queues: queues.clone(),
                scanned_at: std::time::Instant::now(),
            });
        }
        return Ok(queues);
    };

    if refresh {
        let mut conn = conn.clone();
        let token = token.clone();
        tokio::spawn(async move {
            let found = discover_queues(&mut conn, &token).await;
            if let Ok(mut guard) = slot.lock() {
                guard.refreshing = false;
                match found {
                    Ok(queues) => {
                        guard.current = Some(QueueRegistry {
                            queues,
                            scanned_at: std::time::Instant::now(),
                        });
                    }
                    Err(e) => tracing::warn!(error = %e, "queue registry refresh failed"),
                }
            }
        });
    }
    Ok(cached.queues)
}

/// SCAN for `*:meta`, then every meta hash in one pipeline:
/// the BullMQ queues (a `version` field) and their schema.
async fn discover_queues(
    conn: &mut RedisConn,
    token: &CancellationToken,
) -> Result<Vec<(QueueRef, BullmqSchema)>, RanchError> {
    let candidates: Vec<(String, QueueRef)> = scan_meta_keys(conn, token)
        .await?
        .into_iter()
        .filter_map(|key| {
            let (prefix, name) = parse_queue_meta_key(&key)?;
            Some((key, QueueRef { prefix, name }))
        })
        .collect();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let mut pipe = redis::pipe();
    for (key, _) in &candidates {
        pipe.cmd("HGETALL").arg(key);
    }
    let hashes: Vec<HashMap<String, String>> = tokio::select! {
        r = pipe.query_async(conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::InvalidInput {
                reason: "Operation cancelled".to_string(),
            });
        }
    };
    Ok(candidates
        .into_iter()
        .zip(hashes)
        // Skip non-BullMQ `*:meta` hashes (no `version` field, or empty).
        .filter(|(_, hash)| hash.contains_key("version"))
        .map(|((_, qref), hash)| (qref, detect_schema(&hash)))
        .collect())
}

/// Iterate `SCAN MATCH *:meta` until cursor returns 0, collecting all
/// matching keys. Bounded by the actor's cancellation token.
async fn scan_meta_keys(
    conn: &mut RedisConn,
    token: &CancellationToken,
) -> Result<Vec<String>, RanchError> {
    let mut cursor: u64 = 0;
    let mut out: Vec<String> = Vec::new();
    loop {
        let mut scan_cmd = redis::cmd("SCAN");
        scan_cmd
            .arg(cursor)
            .arg("MATCH")
            .arg("*:meta")
            .arg("COUNT")
            // One round trip per COUNT keys: 100 made a large
            // keyspace cost thousands of round trips.
            .arg(SCAN_COUNT);
        let (next, keys): (u64, Vec<String>) = tokio::select! {
            r = scan_cmd.query_async(&mut *conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => {
                return Err(RanchError::InvalidInput {
                    reason: "Operation cancelled".to_string(),
                });
            }
        };
        out.extend(keys);
        cursor = next;
        if cursor == 0 {
            break;
        }
    }
    Ok(out)
}

/// Parse a `bull:emails:meta` key into `(prefix, name)`. Last segment MUST
/// be `"meta"`. We split on the LAST `:` before the `:meta` suffix to
/// extract the queue name; everything before is the prefix.
///
/// Honors custom prefixes:
///   `bull:emails:meta`              → ("bull", "emails")
///   `myapp:queues:billing:meta`     → ("myapp:queues", "billing")
///   `bull:meta`                     → None (no name segment)
///   `:meta`                         → None (empty everywhere)
///   `bull:emails`                   → None (no `:meta` suffix)
fn parse_queue_meta_key(key: &str) -> Option<(String, String)> {
    let trimmed = key.strip_suffix(":meta")?;
    let last_colon = trimmed.rfind(':')?;
    let prefix = &trimmed[..last_colon];
    let name = &trimmed[last_colon + 1..];
    if prefix.is_empty() || name.is_empty() {
        return None;
    }
    Some((prefix.to_string(), name.to_string()))
}

/// First-success-wins schema cache. If `state.cached_schema` already holds
/// V5 or V6, returns it. Otherwise picks the first non-Unknown variant
/// from `detected`, caches it, and returns. If every detected queue has
/// an Unknown schema, returns a typed `SchemaNotRecognized` error
/// carrying the first detected version string for the UI to surface.
fn pick_or_cache_schema(
    cached: &mut Option<BullmqSchema>,
    detected: &[(QueueRef, BullmqSchema)],
) -> Result<BullmqSchema, RanchError> {
    if let Some(s) = cached.clone() {
        if !matches!(s, BullmqSchema::Unknown { .. }) {
            return Ok(s);
        }
    }
    for (_, schema) in detected {
        if !matches!(schema, BullmqSchema::Unknown { .. }) {
            *cached = Some(schema.clone());
            return Ok(schema.clone());
        }
    }
    // Every detected queue had Unknown — surface the first detected_version.
    let first_unknown = detected
        .iter()
        .find_map(|(_, s)| match s {
            BullmqSchema::Unknown { detected_version } => Some(detected_version.clone()),
            _ => None,
        })
        .unwrap_or(None);
    Err(RanchError::SchemaNotRecognized {
        detected_version: first_unknown,
    })
}

/// Keys looked at per SCAN round trip.
const SCAN_COUNT: u32 = 1_000;

/// Replies per queue in [`handle_list_queues`]'s pipeline.
const COUNTER_READS: usize = 9;

/// A queue's counters, pause flag and existence, appended to a pipeline
/// ([`COUNTER_READS`] replies, read by [`counters_from`]).
///
/// "is it paused?" is BullMQ's `meta.paused` flag, not the
/// existence of the `paused` list: that list only exists when jobs were
/// waiting at pause time, so an empty paused queue read as running.
fn push_counter_reads(pipe: &mut redis::Pipeline, keys: KeyVariant, qref: &QueueRef) {
    pipe.cmd("LLEN")
        .arg(keys.wait(qref))
        .cmd("LLEN")
        .arg(keys.active(qref))
        .cmd("ZCARD")
        .arg(keys.completed(qref))
        .cmd("ZCARD")
        .arg(keys.failed(qref))
        .cmd("ZCARD")
        .arg(keys.delayed(qref))
        .cmd("ZCARD")
        .arg(keys.prioritized(qref))
        .cmd("LLEN")
        .arg(keys.paused(qref))
        .cmd("HEXISTS")
        .arg(keys.meta(qref))
        .arg("paused")
        .cmd("EXISTS")
        .arg(keys.meta(qref));
}

/// Read [`push_counter_reads`]'s replies. `None` when the queue's meta is
/// gone (the queue was obliterated since the registry's SCAN).
fn counters_from(qref: &QueueRef, r: &[redis::Value]) -> Result<Option<Queue>, redis::RedisError> {
    let n = |i: usize| -> Result<u64, redis::RedisError> {
        Ok(redis::from_redis_value(
            r.get(i).cloned().unwrap_or(redis::Value::Nil),
        )?)
    };
    let flag = |i: usize| -> Result<bool, redis::RedisError> {
        Ok(redis::from_redis_value(
            r.get(i).cloned().unwrap_or(redis::Value::Nil),
        )?)
    };
    if r.len() < COUNTER_READS || !flag(8)? {
        return Ok(None);
    }
    Ok(Some(Queue {
        queue_ref: qref.clone(),
        counters: QueueCounters {
            waiting: n(0)?,
            active: n(1)?,
            completed: n(2)?,
            failed: n(3)?,
            delayed: n(4)?,
            prioritized: n(5)?,
            paused: n(6)?,
        },
        is_paused: flag(7)?,
    }))
}

/// Map a redis-rs error into a typed `RanchError`. Generic system-error
/// variant — the structured `TestConnectionError` enum is for ping/test
/// only; queue ops surface the verbatim error for the UI.
fn redis_to_ranch_err(e: redis::RedisError) -> RanchError {
    RanchError::Redis(e)
}

// ─────────────────────────────────────────────────────────────────────────────
// handle_list_jobs + helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve the `KeyVariant` from cached schema, or detect on the fly via
/// the queue's `:meta` HGETALL (mirrors the schema discovery in
/// [`handle_list_queues`]). Returns `SchemaNotRecognized` if the queue
/// uses an unsupported BullMQ version.
async fn resolve_keys_for_queue(
    state: &mut ActorState,
    conn: &mut RedisConn,
    qref: &QueueRef,
    token: &CancellationToken,
) -> Result<KeyVariant, RanchError> {
    // Reuse the cached schema if a previous list_queues / list_jobs
    // already resolved it.
    let cached = lock(&state.cached_schema).clone();
    if let Some(cached) = cached {
        if let Some(v) = key_variant_for(&cached) {
            return Ok(v);
        }
    }

    // Detect on the fly: HGETALL the queue's meta hash. We use V5Keys to
    // build the meta-key string — `:meta` is identical across V5 and V6
    // so picking either is fine (verified in the Keys tests).
    let meta_key = V5Keys.meta(qref);
    let mut hgetall_cmd = redis::cmd("HGETALL");
    hgetall_cmd.arg(&meta_key);
    let hash: HashMap<String, String> = tokio::select! {
        r = hgetall_cmd.query_async(&mut *conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::InvalidInput {
                reason: "Operation cancelled".to_string(),
            });
        }
    };
    if hash.is_empty() {
        return Err(RanchError::SchemaNotRecognized {
            detected_version: None,
        });
    }
    let detected = detect_schema(&hash);
    *lock(&state.cached_schema) = Some(detected.clone());
    key_variant_for(&detected).ok_or(RanchError::SchemaNotRecognized {
        detected_version: match detected {
            BullmqSchema::Unknown { detected_version } => detected_version,
            _ => None,
        },
    })
}

fn key_variant_for(schema: &BullmqSchema) -> Option<KeyVariant> {
    match schema {
        BullmqSchema::V5 => Some(KeyVariant::V5(V5Keys)),
        BullmqSchema::V6 => Some(KeyVariant::V6(V6Keys)),
        BullmqSchema::Unknown { .. } => None,
    }
}

/// Pick the right Redis key for a given state via the cached `KeyVariant`.
fn key_for_state(keys: KeyVariant, qref: &QueueRef, state: JobState) -> String {
    match state {
        JobState::Waiting => keys.wait(qref),
        JobState::Active => keys.active(qref),
        JobState::Completed => keys.completed(qref),
        JobState::Failed => keys.failed(qref),
        JobState::Delayed => keys.delayed(qref),
        JobState::Prioritized => keys.prioritized(qref),
        JobState::WaitingChildren => keys.waiting_children(qref),
        JobState::Stalled => keys.stalled(qref),
        JobState::Paused => keys.paused(qref),
        // Unknown shouldn't reach here — handle_list_jobs short-circuits
        // earlier — but we return a never-existing key so an accidental
        // call returns empty rather than panicking.
        JobState::Unknown => format!("{}:{}::ranch-unknown-state", qref.prefix, qref.name),
    }
}

/// Discover + decode a paginated slice of jobs for a (queue,
/// state) pair. See module-level docs for the contract.
async fn handle_list_jobs(
    state: &mut ActorState,
    args: &ListJobsArgs,
    ticket: Option<&QueryTicket>,
    token: &CancellationToken,
) -> Result<ListJobsOutput, RanchError> {
    // Typed three more letters while this waited in the
    // mailbox: don't even open the scan.
    if ticket.is_some_and(QueryTicket::is_superseded) {
        return Err(RanchError::Superseded);
    }
    // SSH tunnel is now handled inside `open_redis_for_actor`.
    let mut conn = open_redis_for_actor(state, token).await?;

    // Resolve the schema → KeyVariant (cached from prior list_queues, or
    // detect on the fly via the queue's meta).
    let keys = resolve_keys_for_queue(state, &mut conn, &args.queue_ref, token).await?;
    let key = key_for_state(keys, &args.queue_ref, args.state);

    // Detect whether we're reading from a list (LRANGE) or sorted set
    // (ZRANGE/ZREVRANGE). Unsupported states return empty quickly.
    let shape = args.state.redis_shape();
    if shape == StateRedisShape::Unsupported {
        return Ok(ListJobsOutput {
            jobs: Vec::new(),
            total: 0,
            state: args.state,
            has_more: false,
            scanned: None,
            scan_capped: false,
        });
    }

    // A search, a reason filter or a custom sort has to look at
    // the jobs themselves: bounded scan instead of a range read.
    let query = JobQuery {
        search: args.search.clone(),
        reason: args.reason.clone(),
        sort: args.sort,
    };
    if query.is_active() {
        return list_jobs_by_query(&mut conn, keys, &key, shape, args, &query, ticket, token).await;
    }

    // Page slice.
    let limit = args.limit.max(1);
    let offset = args.offset;
    let stop = offset + limit - 1; // inclusive in Redis range syntax

    // The total and the page in one round trip. ZREVRANGE for
    // newest-first (most-recent timestamp first), ZRANGE otherwise.
    let (count_cmd, range_cmd) = match shape {
        StateRedisShape::List => ("LLEN", "LRANGE"),
        _ if args.newest_first => ("ZCARD", "ZREVRANGE"),
        _ => ("ZCARD", "ZRANGE"),
    };
    let mut pipe = redis::pipe();
    pipe.cmd(count_cmd)
        .arg(&key)
        .cmd(range_cmd)
        .arg(&key)
        .arg(offset as i64)
        .arg(stop as i64);
    let (total, raw_ids): (u64, Vec<String>) = tokio::select! {
        r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };

    // For v6 list states with marker, subtract 1.
    let v6_marker_state = matches!(*lock(&state.cached_schema), Some(BullmqSchema::V6))
        && shape == StateRedisShape::List
        && matches!(
            args.state,
            JobState::Waiting | JobState::Prioritized | JobState::Delayed
        );
    let total_adjusted = if v6_marker_state && total > 0 {
        total - 1
    } else {
        total
    };

    // Filter out the BullMQ v6 marker token. The marker only appears in
    // v6 list states; passing the bytes-equal check is safe everywhere.
    let job_ids: Vec<String> = raw_ids
        .into_iter()
        .filter(|id| id.as_bytes() != MARKER_TOKEN)
        .collect();

    let jobs = fetch_jobs_by_ids(&mut conn, keys, args, &job_ids, token).await?;

    let returned = jobs.len() as u64;
    let has_more = offset + returned < total_adjusted;

    Ok(ListJobsOutput {
        jobs,
        total: total_adjusted,
        state: args.state,
        has_more,
        scanned: None,
        scan_capped: false,
    })
}

/// `HGETALL` + decode the given ids, in order. Per-job resilience: a job
/// deleted mid-flight or failing to decode is skipped with a warning.
async fn fetch_jobs_by_ids(
    conn: &mut RedisConn,
    keys: KeyVariant,
    args: &ListJobsArgs,
    job_ids: &[String],
    token: &CancellationToken,
) -> Result<Vec<Job>, RanchError> {
    // Build the per-job HGETALL pipeline (non-atomic — independent reads,
    // we just want pipelining round-trip reduction).
    let job_hash_keys: Vec<String> = job_ids
        .iter()
        .map(|id| {
            let jref = JobRef {
                queue: args.queue_ref.clone(),
                id: id.clone(),
            };
            keys.job_hash(&jref)
        })
        .collect();

    // A running job's lock: how long until it expires (the
    // worker renews it while the job runs). Read with the
    // hashes, in the same round trip.
    let with_locks = args.state == JobState::Active;
    let mut pipe = redis::pipe();
    for hk in &job_hash_keys {
        pipe.cmd("HGETALL").arg(hk);
    }
    if with_locks {
        for hk in &job_hash_keys {
            pipe.cmd("PTTL").arg(format!("{hk}:lock"));
        }
    }
    let mut replies: Vec<redis::Value> = if job_hash_keys.is_empty() {
        Vec::new()
    } else {
        tokio::select! {
            r = pipe.query_async(&mut *conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => return Err(RanchError::InvalidInput {
                reason: "Operation cancelled".to_string(),
            }),
        }
    };
    let lock_ttls: Vec<Option<i64>> = if with_locks {
        replies
            .split_off(job_hash_keys.len().min(replies.len()))
            .into_iter()
            .map(|v| redis::from_redis_value::<i64>(v).ok().filter(|t| *t >= 0))
            .collect()
    } else {
        Vec::new()
    };
    let raw_hashes: Vec<HashMap<String, redis::Value>> = replies
        .into_iter()
        .map(|v| redis::from_redis_value(v).unwrap_or_default())
        .collect();

    // Decode each hash via the cached schema's JobDecoder. Per-job
    // resilience: empty hash (job deleted mid-flight) → skip + warn;
    // decode error → skip + warn.
    let decoder: Box<dyn JobDecoder + Send + Sync> = match keys {
        KeyVariant::V5(_) => Box::new(V5Decoder),
        KeyVariant::V6(_) => Box::new(V6Decoder),
    };
    let mut jobs: Vec<Job> = Vec::with_capacity(job_ids.len());
    for (i, (id, raw)) in job_ids.iter().zip(raw_hashes).enumerate() {
        if raw.is_empty() {
            tracing::warn!(
                queue = %args.queue_ref.name,
                job_id = %id,
                "job hash empty (deleted mid-flight); skipping"
            );
            continue;
        }
        let fields = redis_value_hash_to_bytes(raw);
        let jref = JobRef {
            queue: args.queue_ref.clone(),
            id: id.clone(),
        };
        match decoder.decode_job(jref, fields) {
            Ok(mut job) => {
                // Override the heuristic state with the queried state —
                // the membership-query is the authoritative source.
                job.state = args.state;
                job.lock_ttl_ms = lock_ttls.get(i).copied().flatten();
                jobs.push(job);
            }
            Err(e) => {
                tracing::warn!(
                    queue = %args.queue_ref.name,
                    job_id = %id,
                    error = %e,
                    "job decode failed; skipping"
                );
            }
        }
    }

    Ok(jobs)
}

/// Search / sort across a whole state. The deciding
/// logic is pure (`bullmq_schema::job_query`); this fetches:
///
///   1. the newest `SCAN_CAP` ids of the state, in its natural order;
///   2. `HMGET name timestamp processedOn finishedOn failedReason` for each,
///      in pipelines of `SCAN_CHUNK` — cancellable between chunks;
///   3. the full hash of the jobs on the requested page only.
///
/// A newer query on the same connection stops this one between two chunks
/// (`RanchError::Superseded`).
#[allow(clippy::too_many_arguments)]
async fn list_jobs_by_query(
    conn: &mut RedisConn,
    keys: KeyVariant,
    key: &str,
    shape: StateRedisShape,
    args: &ListJobsArgs,
    query: &JobQuery,
    ticket: Option<&QueryTicket>,
    token: &CancellationToken,
) -> Result<ListJobsOutput, RanchError> {
    let cancelled = || RanchError::InvalidInput {
        reason: "Operation cancelled".to_string(),
    };
    let state_total = fetch_total(conn, key, shape, token).await?;

    // +1: a v6 marker token may take one slot of a list state.
    let stop = job_query::SCAN_CAP as i64;
    let cmd_name = match shape {
        StateRedisShape::List => "LRANGE",
        StateRedisShape::SortedSet if args.newest_first => "ZREVRANGE",
        StateRedisShape::SortedSet => "ZRANGE",
        StateRedisShape::Unsupported => unreachable!("short-circuited by handle_list_jobs"),
    };
    let mut cmd = redis::cmd(cmd_name);
    cmd.arg(key).arg(0).arg(stop);
    let raw_ids: Vec<String> = tokio::select! {
        r = cmd.query_async::<Vec<String>>(&mut *conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(cancelled()),
    };
    let had_marker = raw_ids.iter().any(|id| id.as_bytes() == MARKER_TOKEN);
    let ids: Vec<String> = raw_ids
        .into_iter()
        .filter(|id| id.as_bytes() != MARKER_TOKEN)
        .take(job_query::SCAN_CAP)
        .collect();

    let mut rows: Vec<JobRow> = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(job_query::SCAN_CHUNK) {
        if token.is_cancelled() {
            return Err(cancelled());
        }
        if ticket.is_some_and(QueryTicket::is_superseded) {
            return Err(RanchError::Superseded);
        }
        let mut pipe = redis::pipe();
        for id in chunk {
            let jref = JobRef {
                queue: args.queue_ref.clone(),
                id: id.clone(),
            };
            pipe.cmd("HMGET")
                .arg(keys.job_hash(&jref))
                .arg("name")
                .arg("timestamp")
                .arg("processedOn")
                .arg("finishedOn")
                .arg("failedReason");
        }
        let fetched: Vec<Vec<Option<String>>> = tokio::select! {
            r = pipe.query_async(&mut *conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => return Err(cancelled()),
        };
        for (id, fields) in chunk.iter().zip(fetched) {
            let mut fields = fields.into_iter();
            let mut next = || fields.next().flatten();
            let name = next();
            let timestamp = next().and_then(|v| v.parse::<i64>().ok());
            let processed_on = next().and_then(|v| v.parse::<i64>().ok());
            let finished_on = next().and_then(|v| v.parse::<i64>().ok());
            let failed_reason = next().filter(|r| !r.is_empty());
            // No name and no timestamp = the hash is gone (removed mid-scan).
            if name.is_none() && timestamp.is_none() {
                continue;
            }
            rows.push(JobRow {
                id: id.clone(),
                name: name.unwrap_or_default(),
                timestamp,
                processed_on,
                finished_on,
                failed_reason,
            });
        }
    }

    let scanned = rows.len() as u64;
    let (page_ids, matching) = query.run(rows, args.offset, args.limit);
    let jobs = fetch_jobs_by_ids(conn, keys, args, &page_ids, token).await?;
    let has_more = args.offset + (page_ids.len() as u64) < matching;
    let state_jobs = state_total.saturating_sub(u64::from(had_marker));

    Ok(ListJobsOutput {
        jobs,
        total: matching,
        state: args.state,
        has_more,
        scanned: Some(scanned),
        scan_capped: state_jobs > job_query::SCAN_CAP as u64,
    })
}

/// The BullMQ workers attached to a queue. BullMQ keeps
/// no registry: a worker is a Redis connection named after the queue, so
/// this is `CLIENT LIST` + the pure matcher in `bullmq_schema::workers`.
///
/// A provider that forbids `CLIENT` is a capability, not an error: the
/// refusal is cached on the actor and reported as `supported: false`.
/// Network / auth failures stay errors and are not cached.
async fn handle_list_workers(
    state: &mut ActorState,
    args: &ListWorkersArgs,
    token: &CancellationToken,
) -> Result<QueueWorkers, RanchError> {
    match client_list(state, token).await? {
        Ok(raw) => Ok(QueueWorkers {
            supported: true,
            unsupported_reason: None,
            workers: workers::workers_for_queue(&raw, &args.queue_ref),
        }),
        Err(reason) => Ok(QueueWorkers::unsupported(reason)),
    }
}

/// Worker counts of many queues from one `CLIENT LIST`; a
/// Redis that refuses the command answers `supported: false`.
async fn handle_count_queue_workers(
    state: &mut ActorState,
    args: &CountQueueWorkersArgs,
    token: &CancellationToken,
) -> Result<QueueWorkerCounts, RanchError> {
    match client_list(state, token).await? {
        Ok(raw) => Ok(QueueWorkerCounts {
            supported: true,
            counts: workers::count_workers(&raw, &args.queue_refs),
        }),
        Err(_) => Ok(QueueWorkerCounts {
            supported: false,
            counts: Vec::new(),
        }),
    }
}

/// The raw `CLIENT LIST` reply, or `Err(reason)` when this Redis refuses the
/// command (cached on the actor: a provider that forbids it is not asked
/// again). Network / auth failures stay errors and are not cached.
async fn client_list(
    state: &mut ActorState,
    token: &CancellationToken,
) -> Result<Result<String, String>, RanchError> {
    let denied = lock(&state.client_list_denied).clone();
    if let Some(reason) = denied {
        return Ok(Err(reason));
    }
    // A reply that arrived after this call was made is as
    // fresh as reading again: the calls of one burst share one read, and a
    // later call always reads.
    let asked_at = std::time::Instant::now();
    let cache = Arc::clone(&state.client_list_cache);
    let mut cached = cache.lock().await;
    if let Some((arrived_at, raw)) = cached.as_ref() {
        if *arrived_at >= asked_at {
            return Ok(Ok(raw.clone()));
        }
    }
    let mut conn = open_redis_for_actor(state, token).await?;
    let mut cmd = redis::cmd("CLIENT");
    cmd.arg("LIST");
    let reply: Result<String, redis::RedisError> = tokio::select! {
        r = cmd.query_async::<String>(&mut conn) => r,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    match reply {
        Ok(raw) => {
            *cached = Some((std::time::Instant::now(), raw.clone()));
            Ok(Ok(raw))
        }
        Err(e) if workers::client_list_denied(&e.to_string()) => {
            let reason = e.to_string();
            tracing::info!(
                connection = %state.cfg.connection_name,
                %reason,
                "CLIENT LIST refused by this Redis; workers view disabled for the connection"
            );
            *lock(&state.client_list_denied) = Some(reason.clone());
            Ok(Err(reason))
        }
        Err(e) => Err(redis_to_ranch_err(e)),
    }
}

/// What the failed views read: the queues' exact
/// failed counts and the newest failed jobs, bounded (`bullmq_schema::failed`).
///
///   1. the queues (`handle_list_queues` — also caches the schema);
///   2. per queue with failures: the newest `PER_QUEUE_CAP` ids of its
///      `failed` set, with their scores — one pipeline;
///   3. `HMGET name failedReason` for every candidate — one pipeline (the
///      summaries and the text filters need them).
///
/// A queue whose failed set cannot be read is reported in
/// `unreadable_queues`, never fatal.
struct FailedScan {
    counts: Vec<crate::core::bullmq_schema::failed::FailedQueueCount>,
    total_failed: u32,
    raw: Vec<(QueueRef, KeyVariant, String, i64)>,
    candidates: Vec<crate::core::bullmq_schema::failed::FailedCandidate>,
    scan_capped: bool,
    unreadable_queues: Vec<String>,
}

async fn scan_failed(
    state: &mut ActorState,
    only_queue: Option<&str>,
    token: &CancellationToken,
) -> Result<Option<(FailedScan, RedisConn)>, RanchError> {
    use crate::core::bullmq_schema::failed::{
        merge_candidates, FailedCandidate, FailedQueueCount, PER_QUEUE_CAP,
    };
    let cancelled = || RanchError::InvalidInput {
        reason: "Operation cancelled".to_string(),
    };

    // 1. Queues + exact failed counts.
    let queues = handle_list_queues(state, token).await?;
    let mut counts: Vec<FailedQueueCount> = queues
        .iter()
        .filter(|q| q.counters.failed > 0)
        .filter(|q| only_queue.is_none_or(|name| q.queue_ref.name == name))
        .map(|q| FailedQueueCount {
            queue_ref: q.queue_ref.clone(),
            failed: q.counters.failed.min(u32::MAX as u64) as u32,
        })
        .collect();
    counts.sort_by(|a, b| {
        b.failed
            .cmp(&a.failed)
            .then(a.queue_ref.name.cmp(&b.queue_ref.name))
    });
    let total_failed: u32 = counts.iter().map(|c| c.failed).sum();
    if counts.is_empty() {
        return Ok(None);
    }

    let mut conn = open_redis_for_actor(state, token).await?;

    // 2. Newest failed ids per queue — every queue at once.
    let mut unreadable_queues = Vec::new();
    let mut raw: Vec<(QueueRef, KeyVariant, String, i64)> = Vec::new();
    let mut scan_capped = false;
    let mut readable: Vec<(&FailedQueueCount, KeyVariant)> = Vec::new();
    for count in &counts {
        match resolve_keys_for_queue(state, &mut conn, &count.queue_ref, token).await {
            Ok(keys) => readable.push((count, keys)),
            Err(_) => unreadable_queues.push(count.queue_ref.name.clone()),
        }
    }
    let reads = readable
        .iter()
        .map(|(count, keys)| keys.failed(&count.queue_ref))
        .collect();
    let replies = read_each(&conn, reads, token, |mut conn, key: String| async move {
        let mut cmd = redis::cmd("ZREVRANGE");
        cmd.arg(key)
            .arg(0_i64)
            .arg(PER_QUEUE_CAP as i64 - 1)
            .arg("WITHSCORES");
        cmd.query_async::<Vec<(String, f64)>>(&mut conn).await
    })
    .await?;
    for ((count, keys), ids) in readable.into_iter().zip(replies) {
        let q = &count.queue_ref;
        let keys = &keys;
        match ids.unwrap_or_else(|| Err(read_panicked())) {
            Ok(ids) => {
                if count.failed as usize > ids.len() {
                    scan_capped = true;
                }
                for (id, score) in ids {
                    raw.push((q.clone(), *keys, id, score as i64));
                }
            }
            Err(_) => unreadable_queues.push(q.name.clone()),
        }
    }

    // 3. name + failedReason of every candidate.
    let mut candidates: Vec<FailedCandidate> = Vec::with_capacity(raw.len());
    if !raw.is_empty() {
        let mut pipe = redis::pipe();
        for (q, keys, id, _) in &raw {
            pipe.cmd("HMGET")
                .arg(keys.job_hash(&JobRef {
                    queue: q.clone(),
                    id: id.clone(),
                }))
                .arg("name")
                .arg("failedReason");
        }
        let rows: Vec<Vec<Option<String>>> = tokio::select! {
            r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => return Err(cancelled()),
        };
        for ((q, _, id, score), fields) in raw.iter().zip(rows) {
            // No name = the hash is gone (removed mid-flight): not a failed job anymore.
            let Some(name) = fields.first().cloned().flatten() else {
                continue;
            };
            candidates.push(FailedCandidate {
                queue: q.clone(),
                job_id: id.clone(),
                failed_at: *score,
                name,
                reason: fields.get(1).cloned().flatten().filter(|s| !s.is_empty()),
            });
        }
    }
    let before_cap = candidates.len();
    let candidates = merge_candidates(candidates);
    if candidates.len() < before_cap {
        scan_capped = true;
    }
    Ok(Some((
        FailedScan {
            counts,
            total_failed,
            raw,
            candidates,
            scan_capped,
            unreadable_queues,
        },
        conn,
    )))
}

/// Everything that failed on this connection, newest first: the scan ([`scan_failed`]), then `HGETALL` for the jobs of the
/// requested page only.
async fn handle_list_failed_jobs(
    state: &mut ActorState,
    args: &ListFailedJobsArgs,
    token: &CancellationToken,
) -> Result<FailedJobsPage, RanchError> {
    use crate::core::bullmq_schema::failed::{
        page_of, summarise_types, FailedCandidate, FailedFilter,
    };
    let cancelled = || RanchError::InvalidInput {
        reason: "Operation cancelled".to_string(),
    };
    let Some((scan, mut conn)) = scan_failed(state, None, token).await? else {
        return Ok(FailedJobsPage::default());
    };
    let FailedScan {
        counts,
        total_failed,
        raw,
        candidates,
        scan_capped,
        unreadable_queues,
    } = scan;

    let filter = FailedFilter {
        queue: args.queue.clone(),
        name: args.name.clone(),
        reason: args.reason.clone(),
        signature: args.signature.clone(),
    };
    let types = summarise_types(
        &candidates,
        args.queue
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    );
    let (page, total) = page_of(&candidates, &filter, args.offset, args.limit);

    // 4. Full jobs for the page only.
    let mut jobs: Vec<Job> = Vec::with_capacity(page.len());
    if !page.is_empty() {
        let key_of = |c: &FailedCandidate| {
            raw.iter()
                .find(|(q, _, id, _)| q == &c.queue && id == &c.job_id)
                .map(|(_, keys, _, _)| *keys)
        };
        let mut pipe = redis::pipe();
        let mut wanted: Vec<(&FailedCandidate, KeyVariant)> = Vec::new();
        for c in &page {
            if let Some(keys) = key_of(c) {
                pipe.cmd("HGETALL").arg(keys.job_hash(&JobRef {
                    queue: c.queue.clone(),
                    id: c.job_id.clone(),
                }));
                wanted.push((c, keys));
            }
        }
        let hashes: Vec<HashMap<String, redis::Value>> = tokio::select! {
            r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => return Err(cancelled()),
        };
        for ((c, keys), hash) in wanted.into_iter().zip(hashes) {
            if hash.is_empty() {
                continue; // removed between step 3 and step 4
            }
            let decoder: Box<dyn JobDecoder + Send + Sync> = match keys {
                KeyVariant::V5(_) => Box::new(V5Decoder),
                KeyVariant::V6(_) => Box::new(V6Decoder),
            };
            let jref = JobRef {
                queue: c.queue.clone(),
                id: c.job_id.clone(),
            };
            match decoder.decode_job(jref, redis_value_hash_to_bytes(hash)) {
                Ok(mut job) => {
                    job.state = JobState::Failed;
                    jobs.push(job);
                }
                Err(e) => tracing::warn!(
                    queue = %c.queue.name, job_id = %c.job_id, error = %e,
                    "failed-job decode failed; skipping"
                ),
            }
        }
    }

    Ok(FailedJobsPage {
        jobs,
        total,
        total_failed,
        queues: counts,
        types,
        scanned: candidates.len() as u32,
        scan_capped,
        unreadable_queues,
    })
}

/// The failed jobs of a queue (or of the connection) grouped
/// by cause: the same bounded scan as the failed view.
async fn handle_list_failure_causes(
    state: &mut ActorState,
    args: &ListFailureCausesArgs,
    token: &CancellationToken,
) -> Result<FailureCauses, RanchError> {
    use crate::core::bullmq_schema::failed::group_by_cause;
    let queue = args
        .queue
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let Some((scan, _conn)) = scan_failed(state, queue, token).await? else {
        return Ok(FailureCauses::default());
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Ok(FailureCauses {
        causes: group_by_cause(&scan.candidates, queue, now_ms),
        scanned: scan.candidates.len() as u32,
        scan_capped: scan.scan_capped,
        total_failed: scan.total_failed,
        unreadable_queues: scan.unreadable_queues,
    })
}

/// The jobs that ran last, on one queue or across the connection:
///
///   1. the queues (one, or every queue that has something running or finished);
///   2. per queue, one pipeline: the newest ids of `completed` and `failed`
///      with their scores, and the head of `active`;
///   3. `HGETALL` for the `limit` latest finished jobs and the running ones —
///      one pipeline.
///
/// A queue that cannot be read is reported in `unreadable_queues`, never
/// fatal.
async fn handle_list_recent_jobs(
    state: &mut ActorState,
    args: &ListRecentJobsArgs,
    token: &CancellationToken,
) -> Result<RecentJobs, RanchError> {
    use crate::core::bullmq_schema::recent::{
        clamp_limit, newest_finished, order_recent, FinishedCandidate, MAX_ACTIVE,
    };
    let cancelled = || RanchError::InvalidInput {
        reason: "Operation cancelled".to_string(),
    };
    let limit = clamp_limit(args.limit);

    // 1. Which queues.
    let queues: Vec<QueueRef> = match &args.queue_ref {
        Some(q) => vec![q.clone()],
        None => handle_list_queues(state, token)
            .await?
            .into_iter()
            .filter(|q| q.counters.active + q.counters.completed + q.counters.failed > 0)
            .map(|q| q.queue_ref)
            .collect(),
    };
    if queues.is_empty() {
        return Ok(RecentJobs::default());
    }

    let mut conn = open_redis_for_actor(state, token).await?;

    // 2. Newest finished ids + running ids, per queue.
    let mut unreadable_queues = Vec::new();
    let mut finished: Vec<FinishedCandidate> = Vec::new();
    let mut running: Vec<(QueueRef, String)> = Vec::new();
    let mut keys_of: HashMap<QueueRef, KeyVariant> = HashMap::new();
    // Every queue's reads at once.
    let mut readable: Vec<(QueueRef, KeyVariant)> = Vec::new();
    for q in &queues {
        match resolve_keys_for_queue(state, &mut conn, q, token).await {
            Ok(keys) => readable.push((q.clone(), keys)),
            Err(_) => unreadable_queues.push(q.name.clone()),
        }
    }
    type Scored = Vec<(String, f64)>;
    let replies = read_each(
        &conn,
        readable.clone(),
        token,
        |mut conn, (q, keys): (QueueRef, KeyVariant)| async move {
            let mut pipe = redis::pipe();
            for key in [keys.completed(&q), keys.failed(&q)] {
                pipe.cmd("ZREVRANGE")
                    .arg(key)
                    .arg(0_i64)
                    .arg(limit as i64 - 1)
                    .arg("WITHSCORES");
            }
            pipe.cmd("LRANGE")
                .arg(keys.active(&q))
                .arg(0_i64)
                .arg(limit as i64 - 1);
            pipe.query_async::<(Scored, Scored, Vec<String>)>(&mut conn)
                .await
        },
    )
    .await?;
    for ((q, keys), read) in readable.iter().zip(replies) {
        let (q, keys) = (q, *keys);
        let Some(Ok((completed, failed, active))) = read else {
            unreadable_queues.push(q.name.clone());
            continue;
        };
        keys_of.insert(q.clone(), keys);
        for (ids, job_state) in [(completed, JobState::Completed), (failed, JobState::Failed)] {
            finished.extend(ids.into_iter().map(|(id, score)| FinishedCandidate {
                queue: q.clone(),
                job_id: id,
                state: job_state,
                finished_at: score as i64,
            }));
        }
        running.extend(active.into_iter().map(|id| (q.clone(), id)));
    }
    running.truncate(MAX_ACTIVE);

    // 3. Full hashes of what will be shown.
    let mut wanted: Vec<(QueueRef, String, JobState)> = running
        .into_iter()
        .map(|(q, id)| (q, id, JobState::Active))
        .collect();
    wanted.extend(
        newest_finished(finished, limit)
            .into_iter()
            .map(|c| (c.queue, c.job_id, c.state)),
    );
    let mut jobs: Vec<Job> = Vec::with_capacity(wanted.len());
    if !wanted.is_empty() {
        let mut pipe = redis::pipe();
        for (q, id, _) in &wanted {
            pipe.cmd("HGETALL").arg(keys_of[q].job_hash(&JobRef {
                queue: q.clone(),
                id: id.clone(),
            }));
        }
        let hashes: Vec<HashMap<String, redis::Value>> = tokio::select! {
            r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => return Err(cancelled()),
        };
        for ((q, id, job_state), hash) in wanted.into_iter().zip(hashes) {
            if hash.is_empty() {
                continue; // removed (removeOnComplete…) between the two reads
            }
            let decoder: Box<dyn JobDecoder + Send + Sync> = match keys_of[&q] {
                KeyVariant::V5(_) => Box::new(V5Decoder),
                KeyVariant::V6(_) => Box::new(V6Decoder),
            };
            let jref = JobRef {
                queue: q.clone(),
                id: id.clone(),
            };
            match decoder.decode_job(jref, redis_value_hash_to_bytes(hash)) {
                Ok(mut job) => {
                    job.state = job_state; // set membership is the authority
                    jobs.push(job);
                }
                Err(e) => tracing::warn!(
                    queue = %q.name, job_id = %id, error = %e,
                    "recent-job decode failed; skipping"
                ),
            }
        }
    }

    Ok(RecentJobs {
        jobs: order_recent(jobs, limit),
        unreadable_queues,
    })
}

/// Fetch total count for a state via LLEN or ZCARD.
async fn fetch_total(
    conn: &mut RedisConn,
    key: &str,
    shape: StateRedisShape,
    token: &CancellationToken,
) -> Result<u64, RanchError> {
    let cmd_name = match shape {
        StateRedisShape::List => "LLEN",
        StateRedisShape::SortedSet => "ZCARD",
        StateRedisShape::Unsupported => return Ok(0),
    };
    let mut cmd = redis::cmd(cmd_name);
    cmd.arg(key);
    tokio::select! {
        r = cmd.query_async::<u64>(conn) => r.map_err(redis_to_ranch_err),
        _ = token.cancelled() => Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    }
}

/// Convert `HashMap<String, redis::Value>` (what HGETALL returns when we
/// don't pin the value type) into `HashMap<String, Vec<u8>>` — the shape
/// `JobDecoder::decode_job` expects.
fn redis_value_hash_to_bytes(raw: HashMap<String, redis::Value>) -> HashMap<String, Vec<u8>> {
    raw.into_iter()
        .filter_map(|(k, v)| {
            let bytes = match v {
                redis::Value::BulkString(b) => Some(b),
                redis::Value::SimpleString(s) => Some(s.into_bytes()),
                redis::Value::Int(i) => Some(i.to_string().into_bytes()),
                _ => None,
            };
            bytes.map(|b| (k, b))
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// handle_get_job + handle_get_job_logs
// ─────────────────────────────────────────────────────────────────────────────

/// Fetch + decode a single job for the drill-down route.
/// One HGETALL on `bull:{queue}:{id}` + a `JobDecoder::decode_job` call.
/// Returns `RanchError::JobNotFound` on an empty hash so the UI can render
/// an explicit 404-style empty state without retrying.
async fn handle_get_job(
    state: &mut ActorState,
    args: &GetJobArgs,
    token: &CancellationToken,
) -> Result<Job, RanchError> {
    // SSH tunnel is now handled inside `open_redis_for_actor`.
    let mut conn = open_redis_for_actor(state, token).await?;

    let keys = resolve_keys_for_queue(state, &mut conn, &args.queue_ref, token).await?;
    let jref = JobRef {
        queue: args.queue_ref.clone(),
        id: args.job_id.clone(),
    };
    let key = keys.job_hash(&jref);

    // The hash and its lock (a job that started and
    // has not finished may hold one) in one round trip.
    let mut pipe = redis::pipe();
    pipe.cmd("HGETALL")
        .arg(&key)
        .cmd("PTTL")
        .arg(format!("{key}:lock"));
    let (raw, lock_ttl): (HashMap<String, redis::Value>, i64) = tokio::select! {
        r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::InvalidInput {
                reason: "Operation cancelled".to_string(),
            });
        }
    };

    if raw.is_empty() {
        return Err(RanchError::JobNotFound {
            queue: args.queue_ref.name.clone(),
            id: args.job_id.clone(),
        });
    }

    let fields = redis_value_hash_to_bytes(raw);
    let decoder: Box<dyn JobDecoder + Send + Sync> = match keys {
        KeyVariant::V5(_) => Box::new(V5Decoder),
        KeyVariant::V6(_) => Box::new(V6Decoder),
    };
    let mut job = decoder
        .decode_job(jref, fields)
        .map_err(|e| RanchError::InvalidInput {
            reason: format!(
                "Failed to decode job {}/{}: {e}",
                args.queue_ref.name, args.job_id
            ),
        })?;
    if job.processed_on.is_some() && job.finished_on.is_none() {
        job.lock_ttl_ms = (lock_ttl >= 0).then_some(lock_ttl);
    }
    Ok(job)
}

/// Read BullMQ job logs from the `bull:{q}:{id}:logs` list.
/// `Keys::logs(jref)` already gives the right key shape
/// for both v5 and v6 (identical).
async fn handle_get_job_logs(
    state: &mut ActorState,
    args: &GetJobLogsArgs,
    token: &CancellationToken,
) -> Result<JobLogsOutput, RanchError> {
    // SSH tunnel is now handled inside `open_redis_for_actor`.
    let mut conn = open_redis_for_actor(state, token).await?;

    let keys = resolve_keys_for_queue(state, &mut conn, &args.queue_ref, token).await?;
    let jref = JobRef {
        queue: args.queue_ref.clone(),
        id: args.job_id.clone(),
    };
    let key = keys.logs(&jref);

    // LLEN — returns 0 cleanly when the key doesn't exist.
    let mut llen_cmd = redis::cmd("LLEN");
    llen_cmd.arg(&key);
    let total: u64 = tokio::select! {
        r = llen_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::InvalidInput {
                reason: "Operation cancelled".to_string(),
            });
        }
    };

    if total == 0 {
        return Ok(JobLogsOutput {
            lines: Vec::new(),
            total: 0,
            has_more: false,
        });
    }

    let limit = args.limit.max(1);
    let offset = args.offset;
    let stop = offset + limit - 1;

    let mut lrange_cmd = redis::cmd("LRANGE");
    lrange_cmd.arg(&key).arg(offset as i64).arg(stop as i64);
    let raw_lines: Vec<Vec<u8>> = tokio::select! {
        r = lrange_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::InvalidInput {
                reason: "Operation cancelled".to_string(),
            });
        }
    };

    // Lossy UTF-8 — BullMQ stores log lines as UTF-8 strings; the rare
    // embedded-binary case shouldn't fail the whole call.
    let lines: Vec<String> = raw_lines
        .into_iter()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .collect();
    let has_more = offset + (lines.len() as u64) < total;

    Ok(JobLogsOutput {
        lines,
        total,
        has_more,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// handle_list_cron_jobs
// ─────────────────────────────────────────────────────────────────────────────

/// List CRON / repeatable schedulers for a queue.
/// Reads the `bull:{queue}:repeat` ZSET + pipelines HGETALL per
/// scheduler key. Per-scheduler resilience: empty hash or invalid
/// decode → `tracing::warn!` + skip (don't fail the whole call).
async fn handle_list_cron_jobs(
    state: &mut ActorState,
    args: &ListCronJobsArgs,
    token: &CancellationToken,
) -> Result<Vec<CronJob>, RanchError> {
    // SSH tunnel is now handled inside `open_redis_for_actor`.
    let mut conn = open_redis_for_actor(state, token).await?;

    let keys = resolve_keys_for_queue(state, &mut conn, &args.queue_ref, token).await?;
    let repeat_key = keys.repeat(&args.queue_ref);

    // ZRANGE WITHSCORES — returns Vec<(member, score)>.
    let mut zrange_cmd = redis::cmd("ZRANGE");
    zrange_cmd
        .arg(&repeat_key)
        .arg(0_i64)
        .arg(-1_i64)
        .arg("WITHSCORES");
    let members_with_scores: Vec<(String, f64)> = tokio::select! {
        r = zrange_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::InvalidInput {
                reason: "Operation cancelled".to_string(),
            });
        }
    };

    if members_with_scores.is_empty() {
        return Ok(Vec::new());
    }

    // Pipeline HGETALL per scheduler key — non-atomic (independent reads).
    let hash_keys: Vec<String> = members_with_scores
        .iter()
        .map(|(member, _)| format!("{repeat_key}:{member}"))
        .collect();

    let mut pipe = redis::pipe();
    for hk in &hash_keys {
        pipe.cmd("HGETALL").arg(hk);
    }
    let raw_hashes: Vec<HashMap<String, redis::Value>> = tokio::select! {
        r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::InvalidInput {
                reason: "Operation cancelled".to_string(),
            });
        }
    };

    let mut out: Vec<CronJob> = Vec::with_capacity(members_with_scores.len());
    for ((member, score), raw) in members_with_scores.into_iter().zip(raw_hashes) {
        if raw.is_empty() {
            tracing::warn!(
                queue = %args.queue_ref.name,
                scheduler_key = %member,
                "scheduler hash empty (removed mid-flight); skipping"
            );
            continue;
        }
        let fields = redis_value_hash_to_bytes(raw);
        // BullMQ writes scheduler scores as ms-since-epoch integers; redis-rs
        // gives them back as f64 from a WITHSCORES query. Cast safely.
        let next_run_ms = score as i64;
        if let Some(cj) = decode_cron_hash(member, next_run_ms, &fields) {
            out.push(cj);
        }
        // `decode_cron_hash` already emits a `tracing::warn!` on skip cases
        // (missing name, both/neither pattern+every).
    }

    Ok(out)
}

/// The connection's queues' `…:events` streams since each
/// queue's cursor: one pipeline of `XREVRANGE + <cursor> COUNT n` (newest
/// first, capped), then `HGET name` for the jobs whose event does not carry
/// it. Pure part: `bullmq_schema::events`.
async fn handle_read_queue_events(
    state: &mut ActorState,
    args: &ReadQueueEventsArgs,
    token: &CancellationToken,
) -> Result<crate::core::bullmq_schema::events::QueueEvents, RanchError> {
    use crate::core::bullmq_schema::events::{
        after_cursor, merge, to_event, EventCursor, QueueEvents, FIRST_READ, PER_QUEUE_CAP,
    };
    let cancelled = || RanchError::InvalidInput {
        reason: "Operation cancelled".to_string(),
    };
    let limit = args.limit.unwrap_or(100).clamp(1, 500) as usize;
    let queues = handle_list_queues(state, token).await?;
    if queues.is_empty() {
        return Ok(QueueEvents::default());
    }
    let mut conn = open_redis_for_actor(state, token).await?;
    let mut targets = Vec::with_capacity(queues.len());
    for q in &queues {
        if let Ok(keys) = resolve_keys_for_queue(state, &mut conn, &q.queue_ref, token).await {
            let cursor = args
                .cursors
                .iter()
                .find(|c| c.queue_name == q.queue_ref.name)
                .map(|c| c.last_id.clone());
            targets.push((q.queue_ref.clone(), keys, cursor));
        }
    }
    let mut pipe = redis::pipe();
    for (q, keys, cursor) in &targets {
        let (from, count) = match cursor {
            Some(c) => (c.as_str(), PER_QUEUE_CAP + 1),
            None => ("-", FIRST_READ),
        };
        pipe.cmd("XREVRANGE")
            .arg(keys.events(q))
            .arg("+")
            .arg(from)
            .arg("COUNT")
            .arg(count);
    }
    let reads: Vec<Vec<crate::core::bullmq_schema::events::StreamEntry>> = tokio::select! {
        r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(cancelled()),
    };
    let mut events = Vec::new();
    let mut cursors = Vec::with_capacity(targets.len());
    let mut sampled = false;
    for ((q, _, cursor), entries) in targets.iter().zip(reads) {
        let (fresh, overflowed, next) = after_cursor(entries, cursor.as_deref(), PER_QUEUE_CAP + 1);
        sampled |= overflowed;
        if let Some(last_id) = next {
            cursors.push(EventCursor {
                queue_name: q.name.clone(),
                last_id,
            });
        }
        events.extend(
            fresh
                .iter()
                .filter_map(|(id, fields)| to_event(&q.name, id, fields)),
        );
    }
    let (mut events, cut) = merge(events, limit);
    sampled |= cut;

    // Names of the jobs whose event does not carry it (all but `added`).
    let mut wanted: Vec<usize> = Vec::new();
    let mut pipe = redis::pipe();
    for (i, e) in events.iter().enumerate() {
        let (None, Some(job_id)) = (&e.job_name, &e.job_id) else {
            continue;
        };
        if let Some((q, keys, _)) = targets.iter().find(|(q, _, _)| q.name == e.queue_name) {
            pipe.cmd("HGET")
                .arg(keys.job_hash(&JobRef {
                    queue: q.clone(),
                    id: job_id.clone(),
                }))
                .arg("name");
            wanted.push(i);
        }
    }
    if !wanted.is_empty() {
        let names: Result<Vec<Option<String>>, _> = tokio::select! {
            r = pipe.query_async(&mut conn) => r,
            _ = token.cancelled() => return Err(cancelled()),
        };
        // A job removed since keeps its event, without a name.
        if let Ok(names) = names {
            for (&i, name) in wanted.iter().zip(names) {
                events[i].job_name = name;
            }
        }
    }
    Ok(QueueEvents {
        events,
        cursors,
        sampled,
    })
}

/// One timed `PING`, then `INFO` (all sections: a managed
/// Redis that refuses a section still answers the others in the default
/// reply). An `INFO` that fails leaves the fields empty, never the strip.
async fn handle_get_server_info(
    state: &mut ActorState,
    token: &CancellationToken,
) -> Result<crate::core::redis_info::ServerInfo, RanchError> {
    let cancelled = || RanchError::InvalidInput {
        reason: "Operation cancelled".to_string(),
    };
    let mut conn = open_redis_for_actor(state, token).await?;
    let ping = redis::cmd("PING");
    let started = std::time::Instant::now();
    let _: String = tokio::select! {
        r = ping.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(cancelled()),
    };
    let latency_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
    let info_cmd = redis::cmd("INFO");
    let info: Result<String, _> = tokio::select! {
        r = info_cmd.query_async(&mut conn) => r,
        _ = token.cancelled() => return Err(cancelled()),
    };
    Ok(crate::core::redis_info::parse_info(
        &info.unwrap_or_default(),
        latency_ms,
    ))
}

/// Completed / failed over time for every queue of the
/// connection, from native metrics only: one pipeline of `HMGET` + `LRANGE`
/// per queue and kind. A queue whose keys cannot be resolved is left out.
async fn handle_get_throughput(
    state: &mut ActorState,
    args: &GetThroughputArgs,
    token: &CancellationToken,
) -> Result<crate::core::bullmq_schema::Throughput, RanchError> {
    use crate::core::bullmq_schema::metrics::{throughput, NativeMetrics};
    let cancelled = || RanchError::InvalidInput {
        reason: "Operation cancelled".to_string(),
    };
    let queues = handle_list_queues(state, token).await?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    if queues.is_empty() {
        return Ok(throughput(args.range, now_ms, Vec::new()));
    }
    let mut conn = open_redis_for_actor(state, token).await?;
    let mut resolved = Vec::with_capacity(queues.len());
    for q in &queues {
        if let Ok(keys) = resolve_keys_for_queue(state, &mut conn, &q.queue_ref, token).await {
            resolved.push((q.queue_ref.clone(), keys));
        }
    }
    let minutes = args.range.window_minutes();
    let mut pipe = redis::pipe();
    for (q, keys) in &resolved {
        for kind in ["completed", "failed"] {
            pipe.cmd("HMGET")
                .arg(keys.metrics_meta(q, kind))
                .arg("count")
                .arg("prevCount")
                .arg("prevTS");
            pipe.cmd("LRANGE")
                .arg(keys.metrics_data(q, kind))
                .arg(0_i64)
                .arg(minutes);
        }
    }
    let raw: Vec<redis::Value> = tokio::select! {
        r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(cancelled()),
    };
    let mut it = raw.into_iter();
    let mut native = || -> NativeMetrics {
        let meta: Vec<Option<String>> =
            redis::from_redis_value(it.next().unwrap_or(redis::Value::Nil)).unwrap_or_default();
        let data: Vec<String> =
            redis::from_redis_value(it.next().unwrap_or(redis::Value::Nil)).unwrap_or_default();
        let num = |i: usize| -> i64 {
            meta.get(i)
                .and_then(|v| v.as_deref())
                .and_then(|s| s.parse::<f64>().ok())
                .map(|v| v as i64)
                .unwrap_or(0)
        };
        NativeMetrics {
            count: num(0).max(0) as u64,
            prev_count: num(1).max(0) as u64,
            prev_ts_ms: num(2),
            data: data
                .iter()
                .map(|s| s.parse::<f64>().map(|v| v.max(0.0) as u64).unwrap_or(0))
                .collect(),
        }
    };
    let mut per_queue = Vec::with_capacity(resolved.len());
    for (q, _) in resolved {
        let completed = native();
        let failed = native();
        per_queue.push((q, completed, failed));
    }
    Ok(throughput(args.range, now_ms, per_queue))
}

/// The newest parents of every queue (waiting for children, or
/// finished) name their children's queues; the event streams give the added
/// rate; the tail of `wait` the oldest waiting job.
async fn handle_get_topology(
    state: &mut ActorState,
    token: &CancellationToken,
) -> Result<crate::core::bullmq_schema::topology::Topology, RanchError> {
    use crate::core::bullmq_schema::topology::{
        added_rate, links, queue_of_job_key, QueueFlowStats, Topology, EVENTS_PER_QUEUE,
        PARENTS_PER_QUEUE,
    };
    let now_ms = chrono::Utc::now().timestamp_millis();
    let queues = handle_list_queues(state, token).await?;
    let mut conn = open_redis_for_actor(state, token).await?;
    let mut readable: Vec<(QueueRef, KeyVariant)> = Vec::new();
    for q in &queues {
        if let Ok(keys) = resolve_keys_for_queue(state, &mut conn, &q.queue_ref, token).await {
            readable.push((q.queue_ref.clone(), keys));
        }
    }

    // Every queue's first read at once, then every second read.
    type First = (
        Vec<String>,
        Vec<String>,
        Vec<crate::core::bullmq_schema::events::StreamEntry>,
        Option<String>,
    );
    let firsts = read_each(
        &conn,
        readable.clone(),
        token,
        |mut conn, (qref, keys): (QueueRef, KeyVariant)| async move {
            let mut first = redis::pipe();
            first
                .cmd("ZREVRANGE")
                .arg(keys.waiting_children(&qref))
                .arg(0)
                .arg(PARENTS_PER_QUEUE as i64 - 1);
            first
                .cmd("ZREVRANGE")
                .arg(keys.completed(&qref))
                .arg(0)
                .arg(PARENTS_PER_QUEUE as i64 - 1);
            first
                .cmd("XREVRANGE")
                .arg(keys.events(&qref))
                .arg("+")
                .arg("-")
                .arg("COUNT")
                .arg(EVENTS_PER_QUEUE);
            first.cmd("LINDEX").arg(keys.wait(&qref)).arg(-1);
            first.query_async::<First>(&mut conn).await
        },
    )
    .await?;
    let firsts: Vec<First> = firsts
        .into_iter()
        .map(|r| {
            r.unwrap_or_else(|| Err(read_panicked()))
                .map_err(redis_to_ranch_err)
        })
        .collect::<Result<_, _>>()?;

    let seconds = read_each(
        &conn,
        readable
            .iter()
            .zip(&firsts)
            .map(
                |((qref, keys), (waiting_parents, done_parents, _, oldest_id))| {
                    let job = |id: &str| {
                        keys.job_hash(&JobRef {
                            queue: qref.clone(),
                            id: id.to_string(),
                        })
                    };
                    let mut second = redis::pipe();
                    for id in waiting_parents {
                        second
                            .cmd("SMEMBERS")
                            .arg(format!("{}:dependencies", job(id)));
                    }
                    for id in done_parents {
                        second.cmd("HKEYS").arg(format!("{}:processed", job(id)));
                    }
                    if let Some(id) = oldest_id {
                        second.cmd("HGET").arg(job(id)).arg("timestamp");
                    }
                    second
                },
            )
            .collect(),
        token,
        |mut conn, second: redis::Pipeline| async move {
            // Nothing to read (no parent, nothing waiting): an empty pipeline
            // is an error in redis-rs, not an empty reply.
            if second.cmd_iter().next().is_none() {
                return Ok(Vec::new());
            }
            second.query_async::<Vec<redis::Value>>(&mut conn).await
        },
    )
    .await?;

    let mut pairs = Vec::new();
    let mut stats = Vec::new();
    for (((qref, _), (_, _, events, oldest_id)), second) in readable.iter().zip(firsts).zip(seconds)
    {
        let mut replies = second
            .unwrap_or_else(|| Err(read_panicked()))
            .map_err(redis_to_ranch_err)?;
        let oldest_waiting_ms = if oldest_id.is_some() {
            replies
                .pop()
                .and_then(|v| redis::from_redis_value::<Option<String>>(v).ok().flatten())
                .and_then(|t| t.parse::<f64>().ok())
                .map(|t| t as i64)
        } else {
            None
        };
        for reply in replies {
            let children: Vec<String> = redis::from_redis_value(reply).unwrap_or_default();
            for child in children {
                if let Some(child_queue) = queue_of_job_key(&qref.prefix, &child) {
                    pairs.push((qref.clone(), child_queue));
                }
            }
        }
        let entries: Vec<(String, String)> = events
            .into_iter()
            .map(|(id, fields)| {
                let event = fields
                    .chunks(2)
                    .find(|kv| kv.first().is_some_and(|k| k == "event"))
                    .and_then(|kv| kv.get(1).cloned())
                    .unwrap_or_default();
                (id, event)
            })
            .collect();
        let (added_per_minute, added_capped) = added_rate(&entries, now_ms, EVENTS_PER_QUEUE);
        stats.push(QueueFlowStats {
            queue_ref: qref.clone(),
            added_per_minute,
            added_capped,
            oldest_waiting_ms,
        });
    }
    Ok(Topology {
        links: links(pairs),
        queues: stats,
    })
}

/// The jobs that finished in the visible range (the completed
/// and failed sets are scored by finish time), newest first and capped per
/// queue and state, summarized for the dragged window.
async fn handle_summarize_window(
    state: &mut ActorState,
    args: &crate::commands::queues::SummarizeWindowArgs,
    token: &CancellationToken,
) -> Result<crate::core::bullmq_schema::timeline::WindowSummary, RanchError> {
    use crate::core::bullmq_schema::timeline::{summarize, FinishedJob, WINDOW_SCAN_PER_SET};
    let now_ms = chrono::Utc::now().timestamp_millis();
    let queues = handle_list_queues(state, token).await?;
    let mut conn = open_redis_for_actor(state, token).await?;

    // Every (queue, completed|failed) set read at once: its
    // finished ids in the range, then their timings.
    let mut sets: Vec<(QueueRef, KeyVariant, bool)> = Vec::new();
    for q in &queues {
        if let Ok(keys) = resolve_keys_for_queue(state, &mut conn, &q.queue_ref, token).await {
            sets.push((q.queue_ref.clone(), keys, false));
            sets.push((q.queue_ref.clone(), keys, true));
        }
    }
    let range_from_ms = args.range_from_ms;
    type Rows = (Vec<String>, Vec<Vec<Option<String>>>);
    let reads = read_each(
        &conn,
        sets.clone(),
        token,
        move |mut conn, (qref, keys, failed): (QueueRef, KeyVariant, bool)| async move {
            let set = if failed {
                keys.failed(&qref)
            } else {
                keys.completed(&qref)
            };
            let ids: Vec<String> = redis::cmd("ZREVRANGEBYSCORE")
                .arg(&set)
                .arg(now_ms)
                .arg(range_from_ms)
                .arg("LIMIT")
                .arg(0)
                .arg(WINDOW_SCAN_PER_SET)
                .query_async(&mut conn)
                .await?;
            if ids.is_empty() {
                return Ok::<Rows, redis::RedisError>((ids, Vec::new()));
            }
            let mut pipe = redis::pipe();
            for id in &ids {
                pipe.cmd("HMGET")
                    .arg(keys.job_hash(&JobRef {
                        queue: qref.clone(),
                        id: id.clone(),
                    }))
                    .arg("timestamp")
                    .arg("processedOn")
                    .arg("finishedOn")
                    .arg("failedReason");
            }
            let rows = pipe.query_async(&mut conn).await?;
            Ok((ids, rows))
        },
    )
    .await?;

    let mut jobs: Vec<FinishedJob> = Vec::new();
    let mut capped = false;
    for ((qref, _, failed), read) in sets.into_iter().zip(reads) {
        let (ids, rows) = read
            .unwrap_or_else(|| Err(read_panicked()))
            .map_err(redis_to_ranch_err)?;
        capped |= ids.len() >= WINDOW_SCAN_PER_SET;
        for (id, row) in ids.into_iter().zip(rows) {
            let num = |i: usize| -> Option<i64> {
                row.get(i)
                    .and_then(|v| v.as_deref())
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|v| v as i64)
            };
            let Some(finished_on) = num(2) else {
                continue; // removed under our feet
            };
            jobs.push(FinishedJob {
                job_ref: JobRef {
                    queue: qref.clone(),
                    id,
                },
                failed,
                timestamp: num(0).unwrap_or(finished_on),
                processed_on: num(1),
                finished_on,
                failed_reason: row.get(3).cloned().flatten(),
            });
        }
    }
    Ok(summarize(
        &jobs,
        args.from_ms,
        args.to_ms,
        args.range_from_ms,
        now_ms,
        capped,
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// handle_get_queue_metrics (24h observability dashboard)
// ─────────────────────────────────────────────────────────────────────────────

/// Metrics of one queue over a selectable window.
/// All arithmetic lives in `bullmq_schema::metrics`; this
/// handler only fetches, in three pipelined round-trips:
///
///   1. native metrics (`metrics:completed|failed` hash + `:data` list) AND
///      the `ZCOUNT`-per-bucket estimate, in one pipeline;
///   2. the ids of the most recent completed / failed jobs in the window
///      (`ZREVRANGEBYSCORE … LIMIT 0 N`);
///   3. `HMGET` of the fields metrics need, for those ids.
///
/// Counts come from native metrics when the queue records them (exact, and
/// immune to `removeOnComplete`), from the estimate otherwise. Timing
/// figures, slowest jobs and failing types always come from the sample.
async fn handle_get_queue_metrics(
    state: &mut ActorState,
    args: &GetQueueMetricsArgs,
    token: &CancellationToken,
) -> Result<QueueMetrics, RanchError> {
    use crate::core::bullmq_schema::metrics::{
        build, Buckets, JobSample, NativeMetrics, QueueMetricsInput,
    };
    /// Bounded sample per state — enough for stable averages and a top 10,
    /// small enough for one HMGET pipeline.
    const SAMPLE: i64 = 200;

    let cancelled = || RanchError::InvalidInput {
        reason: "Operation cancelled".to_string(),
    };

    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &args.queue_ref, token).await?;
    let q = &args.queue_ref;
    let completed_key = keys.completed(q);
    let failed_key = keys.failed(q);

    let now_ms = args
        .now_ms
        .map(|v| v as i64)
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let buckets = Buckets::new(args.range, now_ms);
    let n = buckets.count();
    let minutes = args.range.window_minutes();

    // ── 1. native metrics + estimate, one round-trip ─────────────────────
    let mut pipe = redis::pipe();
    for kind in ["completed", "failed"] {
        pipe.cmd("HMGET")
            .arg(keys.metrics_meta(q, kind))
            .arg("count")
            .arg("prevCount")
            .arg("prevTS");
        pipe.cmd("LRANGE")
            .arg(keys.metrics_data(q, kind))
            .arg(0_i64)
            .arg(minutes);
    }
    for key in [&completed_key, &failed_key] {
        for i in 0..n {
            let (lo, hi) = buckets.zcount_bounds(i);
            pipe.cmd("ZCOUNT").arg(key).arg(lo).arg(hi);
        }
    }
    type NativeRaw = (Vec<Option<String>>, Vec<String>);
    let (native_completed_raw, native_failed_raw, counts): (NativeRaw, NativeRaw, Vec<u64>) = {
        // The pipeline returns 4 + 2n values; read them positionally.
        let raw: Vec<redis::Value> = tokio::select! {
            r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => return Err(cancelled()),
        };
        let mut it = raw.into_iter();
        let mut take = || it.next().unwrap_or(redis::Value::Nil);
        let decode_native = |meta: redis::Value, data: redis::Value| -> NativeRaw {
            (
                redis::from_redis_value(meta).unwrap_or_default(),
                redis::from_redis_value(data).unwrap_or_default(),
            )
        };
        let completed = decode_native(take(), take());
        let failed = decode_native(take(), take());
        let counts: Vec<u64> = it
            .map(|v| redis::from_redis_value::<u64>(v).unwrap_or(0))
            .collect();
        (completed, failed, counts)
    };
    let to_native = |(meta, data): NativeRaw| -> NativeMetrics {
        let num = |i: usize| -> Option<i64> {
            meta.get(i)
                .and_then(|v| v.as_deref())
                .and_then(|s| s.parse::<f64>().ok())
                .map(|v| v as i64)
        };
        NativeMetrics {
            count: num(0).unwrap_or(0).max(0) as u64,
            prev_count: num(1).unwrap_or(0).max(0) as u64,
            prev_ts_ms: num(2).unwrap_or(0),
            data: data
                .iter()
                .map(|s| s.parse::<f64>().map(|v| v.max(0.0) as u64).unwrap_or(0))
                .collect(),
        }
    };
    let native_completed = to_native(native_completed_raw);
    let native_failed = to_native(native_failed_raw);
    let estimated_completed: Vec<u64> = counts.iter().take(n).copied().collect();
    let estimated_failed: Vec<u64> = counts.iter().skip(n).take(n).copied().collect();

    // ── 2. most recent finished job ids in the window ─────────────────────
    let mut pipe = redis::pipe();
    for key in [&completed_key, &failed_key] {
        pipe.cmd("ZREVRANGEBYSCORE")
            .arg(key)
            .arg(now_ms)
            .arg(buckets.window_start_ms())
            .arg("LIMIT")
            .arg(0_i64)
            .arg(SAMPLE);
    }
    let (completed_ids, failed_ids): (Vec<String>, Vec<String>) = tokio::select! {
        r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(cancelled()),
    };

    // ── 3. the fields metrics need, for those jobs ───────────────────────
    let all_ids: Vec<&String> = completed_ids.iter().chain(failed_ids.iter()).collect();
    let mut samples: Vec<JobSample> = Vec::with_capacity(all_ids.len());
    if !all_ids.is_empty() {
        let mut pipe = redis::pipe();
        for id in &all_ids {
            pipe.cmd("HMGET")
                .arg(keys.job_hash(&JobRef {
                    queue: q.clone(),
                    id: (*id).clone(),
                }))
                .arg("name")
                .arg("timestamp")
                .arg("processedOn")
                .arg("finishedOn")
                .arg("delay")
                .arg("failedReason");
        }
        let rows: Vec<Vec<Option<String>>> = tokio::select! {
            r = pipe.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => return Err(cancelled()),
        };
        for (id, fields) in all_ids.iter().zip(rows) {
            let text = |i: usize| fields.get(i).and_then(|v| v.clone());
            let int = |i: usize| {
                text(i)
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|v| v as i64)
            };
            samples.push(JobSample {
                job_id: (*id).clone(),
                name: text(0).unwrap_or_default(),
                timestamp: int(1),
                processed_on: int(2),
                finished_on: int(3),
                delay: int(4),
                failed_reason: text(5).filter(|s| !s.is_empty()),
            });
        }
    }
    let (completed_sample, failed_sample) = samples.split_at(completed_ids.len());

    Ok(build(QueueMetricsInput {
        queue_name: &q.name,
        buckets: &buckets,
        native_completed: Some(&native_completed),
        native_failed: Some(&native_failed),
        estimated_completed: &estimated_completed,
        estimated_failed: &estimated_failed,
        completed_sample,
        failed_sample,
    }))
}
// ─────────────────────────────────────────────────────────────────────────────
// Destructive single-job actions (retry / remove / promote)
// ─────────────────────────────────────────────────────────────────────────────
//
// All three handlers follow the same prescribed shape:
//
//   1. check_lock_or_err(state.lock_state, ...)? ............... safety gate
//   2. open Redis + resolve KeyVariant
//   3. validate state precondition (retry → failed; promote → delayed;
//      remove → any-state, but EXISTS check on the job hash)
//   4. MULTI/EXEC pipeline for the actual mutation
//   5. write_audit(state, action, target, outcome, ...) ........ audit funnel
//   6. return Ok(result)
//
// The lock check is the FIRST line because a locked connection must
// never touch Redis. The audit write is the LAST step before return
// because the audit row must be fsync'd to disk before the
// user sees the action confirmed.
//
// Locked-connection rejections return BEFORE the audit step — for
// single-action paths we deliberately skip auditing the
// rejection (signal-to-noise on single attempts is low; the bulk
// paths write a consolidated "attempted-on-locked" audit row).

/// The actor's Redis connection: the persistent one when it is open, else
/// opened now (validate CA, build the client, connect), racing the actor's
/// cancellation token.
///
/// When `state.cfg.ssh_tunnel` is `Some`, the connection goes
/// through the cached SSH tunnel ([`ensure_ssh_tunnel`]), to its loopback
/// listener. A failed connect invalidates the tunnel so the next call
/// re-tunnels.
///
/// Opened once and kept: handlers get a clone. It
/// reconnects by itself after a drop; a closed tunnel drops it with the
/// tunnel, so both are opened again.
async fn open_redis_for_actor(
    state: &mut ActorState,
    token: &CancellationToken,
) -> Result<RedisConn, RanchError> {
    let validated_ca = validate_ca_pem(state.cfg.tls_custom_ca.as_deref())?;
    // Held while opening, so concurrent reads wait for this
    // one instead of each opening a connection.
    let mut link = state.link.lock().await;
    if link.ssh_tunnel.as_ref().is_some_and(|t| t.is_closed()) {
        link.ssh_tunnel = None;
    }
    let via = if state.cfg.ssh_tunnel.is_some() {
        Some(ensure_ssh_tunnel(&state.cfg, &mut link).await?)
    } else {
        None
    };
    if let Some(cached) = link.redis.as_ref() {
        if cached.via == via {
            return Ok(cached.conn.clone());
        }
    }
    link.redis = None;

    // SSH-tunneled connections use redis:// (no TLS) over loopback —
    // the SSH session is the encryption layer. validated_ca only
    // applies to the direct path.
    let (url, ca_for_call) = match via {
        Some(local) => (build_redis_url_via_local(&state.cfg, local), None),
        None => (
            build_redis_url_from_config(&state.cfg),
            validated_ca.as_deref(),
        ),
    };
    let client = build_redis_client(&url, ca_for_call).map_err(redis_to_ranch_err)?;

    let result = tokio::select! {
        r = RedisConn::new_with_config(client, connection_manager_config()) => r.map_err(redis_to_ranch_err),
        _ = token.cancelled() => Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };

    match result {
        Ok(conn) => {
            link.redis = Some(CachedRedis {
                conn: conn.clone(),
                via,
            });
            Ok(conn)
        }
        Err(e) => {
            // The failure may be on the SSH side: re-tunnel next time.
            if via.is_some() {
                link.ssh_tunnel = None;
            }
            Err(e)
        }
    }
}

/// Aliased so clippy's `type_complexity` lint stays quiet — the 9
/// pipelined membership probes (3 LPOS + 6 ZSCORE) return a wide
/// tuple by design.
type StateProbe = (
    Option<i64>, // wait (LPOS)
    Option<i64>, // active (LPOS)
    Option<i64>, // paused (LPOS)
    Option<f64>, // completed (ZSCORE)
    Option<f64>, // failed
    Option<f64>, // delayed
    Option<f64>, // prioritized
    Option<f64>, // waiting-children
    Option<f64>, // stalled
);

/// Detect which BullMQ state the job currently sits in, returning a
/// lowercase camelCase string ready for the `InvalidJobState` error
/// payload. Used by the precondition-violation path so the error
/// message reads naturally ("current state is waiting, required failed").
///
/// Implementation: one pipelined RTT covering the 3 list states
/// (LPOS) + 6 sorted-set states (ZSCORE). The first hit wins. Not
/// finding the job in any container returns `"unknown"` — the
/// caller decides whether that's a `JobNotFound` (no hash) or just
/// a mid-state race.
async fn detect_job_state(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    job_id: &str,
    token: &CancellationToken,
) -> Result<String, RanchError> {
    let wait_key = keys.wait(qref);
    let active_key = keys.active(qref);
    let paused_key = keys.paused(qref);
    let completed_key = keys.completed(qref);
    let failed_key = keys.failed(qref);
    let delayed_key = keys.delayed(qref);
    let prioritized_key = keys.prioritized(qref);
    let waiting_children_key = keys.waiting_children(qref);
    let stalled_key = keys.stalled(qref);

    let mut pipe = redis::pipe();
    pipe.cmd("LPOS")
        .arg(&wait_key)
        .arg(job_id)
        .cmd("LPOS")
        .arg(&active_key)
        .arg(job_id)
        .cmd("LPOS")
        .arg(&paused_key)
        .arg(job_id)
        .cmd("ZSCORE")
        .arg(&completed_key)
        .arg(job_id)
        .cmd("ZSCORE")
        .arg(&failed_key)
        .arg(job_id)
        .cmd("ZSCORE")
        .arg(&delayed_key)
        .arg(job_id)
        .cmd("ZSCORE")
        .arg(&prioritized_key)
        .arg(job_id)
        .cmd("ZSCORE")
        .arg(&waiting_children_key)
        .arg(job_id)
        .cmd("ZSCORE")
        .arg(&stalled_key)
        .arg(job_id);
    let (
        lpos_wait,
        lpos_active,
        lpos_paused,
        z_completed,
        z_failed,
        z_delayed,
        z_prioritized,
        z_wc,
        z_stalled,
    ): StateProbe = tokio::select! {
        r = pipe.query_async(conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };

    let state = if lpos_wait.is_some() {
        "waiting"
    } else if lpos_active.is_some() {
        "active"
    } else if lpos_paused.is_some() {
        "paused"
    } else if z_failed.is_some() {
        "failed"
    } else if z_delayed.is_some() {
        "delayed"
    } else if z_completed.is_some() {
        "completed"
    } else if z_prioritized.is_some() {
        "prioritized"
    } else if z_wc.is_some() {
        "waitingChildren"
    } else if z_stalled.is_some() {
        "stalled"
    } else {
        "unknown"
    };
    Ok(state.to_string())
}

/// The write-authorization check of every destructive handler
/// that can be reached by an agent. Replaces the bare `check_lock_or_err`
/// there; the lock stays the rule, a human approval is the one exception:
///
///   - no grant (every UI call, and agent calls when approval mode is `off`)
///     → `check_lock_or_err`, exactly as before;
///   - grant → it must flip `approved → consumed` for THIS connection and
///     THIS tool, exactly once. Then the operation runs even if the
///     connection is session-locked: the approval is the explicit human
///     gesture, scoped to one operation. Anything else — forged, pending,
///     denied, expired, already used, issued for another connection or tool —
///     is `ApprovalInvalid` and nothing runs.
///
/// The actor remains the single enforcement point.
async fn authorize_write(
    state: &ActorState,
    attribution: &AuditAttribution,
    tool: &str,
) -> Result<(), RanchError> {
    let Some(grant) = &attribution.approval else {
        return check_lock_or_err(
            state.lock_state,
            &state.cfg.connection_id,
            &state.cfg.connection_name,
        );
    };
    let valid = grant.tool() == tool
        && crate::core::mcp::approval::consume(
            &state.cfg.pool,
            grant.approval_id(),
            &state.cfg.connection_id,
            tool,
        )
        .await?;
    if valid {
        Ok(())
    } else {
        Err(RanchError::ApprovalInvalid {
            tool: tool.to_string(),
        })
    }
}

/// Build + write an audit entry for a destructive action. Snapshots
/// connection identity from `state.cfg`. Returns the audit error if
/// any — failing the action call (if the audit can't be
/// persisted, the user MUST NOT see the action as confirmed).
async fn write_audit_for_action(
    state: &ActorState,
    action: AuditAction,
    target: AuditTarget,
    outcome: AuditOutcome,
    reason: Option<String>,
    attribution: &AuditAttribution,
) -> Result<(), RanchError> {
    audit_record(
        &state.cfg.pool,
        AuditEntry {
            source: attribution.source,
            action,
            target,
            outcome,
            outcome_reason: reason,
            connection_id: Some(state.cfg.connection_id.clone()),
            connection_name: Some(state.cfg.connection_name.clone()),
            env_color: state.cfg.env_color.clone(),
            agent_client: attribution.agent_client.clone(),
            agent_transport: attribution.agent_transport.clone(),
            approval_id: attribution
                .approval
                .as_ref()
                .map(|g| g.approval_id().to_string()),
        },
    )
    .await
}

/// What one job of a flow looks like in Redis.
struct FlowProbe {
    exists: bool,
    name: String,
    timestamp: Option<i64>,
    duration_ms: Option<i64>,
    failed_reason: Option<String>,
    parent_key: Option<String>,
    /// Raw `opts` and `returnvalue`.
    opts: Option<String>,
    return_value: Option<String>,
    state: JobState,
    /// Keys of its direct children, whatever became of them.
    children: Vec<String>,
    /// Children the parent recorded as processed / as failed — it remembers
    /// them even once `removeOnComplete` has deleted their hash.
    completed: u64,
    failed: u64,
}

/// Commands per job in [`probe_flow_jobs`].
const FLOW_PROBE_WIDTH: usize = 14;

/// Read `refs` in ONE pipeline: fields, state (container
/// membership) and the four children keys. A flow of 200 jobs is a handful
/// of round trips (one per tree level), not 200.
async fn probe_flow_jobs(
    conn: &mut RedisConn,
    keys: KeyVariant,
    refs: &[JobRef],
) -> Result<Vec<FlowProbe>, redis::RedisError> {
    if refs.is_empty() {
        return Ok(Vec::new());
    }
    let mut pipe = redis::pipe();
    for jref in refs {
        let q = &jref.queue;
        let job_key = keys.job_hash(jref);
        pipe.cmd("HMGET")
            .arg(&job_key)
            .arg("name")
            .arg("timestamp")
            .arg("processedOn")
            .arg("finishedOn")
            .arg("failedReason")
            .arg("parentKey")
            .arg("opts")
            .arg("returnvalue");
        pipe.cmd("LPOS").arg(keys.wait(q)).arg(&jref.id);
        pipe.cmd("LPOS").arg(keys.active(q)).arg(&jref.id);
        pipe.cmd("LPOS").arg(keys.paused(q)).arg(&jref.id);
        pipe.cmd("ZSCORE").arg(keys.completed(q)).arg(&jref.id);
        pipe.cmd("ZSCORE").arg(keys.failed(q)).arg(&jref.id);
        pipe.cmd("ZSCORE").arg(keys.delayed(q)).arg(&jref.id);
        pipe.cmd("ZSCORE").arg(keys.prioritized(q)).arg(&jref.id);
        pipe.cmd("ZSCORE")
            .arg(keys.waiting_children(q))
            .arg(&jref.id);
        pipe.cmd("EXISTS").arg(&job_key);
        pipe.cmd("SMEMBERS").arg(format!("{job_key}:dependencies"));
        pipe.cmd("HKEYS").arg(format!("{job_key}:processed"));
        pipe.cmd("HKEYS").arg(format!("{job_key}:failed"));
        pipe.cmd("ZRANGE")
            .arg(format!("{job_key}:unsuccessful"))
            .arg(0)
            .arg(-1);
    }
    let raw: Vec<redis::Value> = pipe.query_async(conn).await?;
    let mut probes = Vec::with_capacity(refs.len());
    for chunk in raw.chunks(FLOW_PROBE_WIDTH) {
        if chunk.len() < FLOW_PROBE_WIDTH {
            break;
        }
        let get = |i: usize| chunk[i].clone();
        let fields: Vec<Option<String>> = redis::from_redis_value(get(0))?;
        let field = |i: usize| fields.get(i).cloned().flatten().filter(|v| !v.is_empty());
        let num = |i: usize| field(i).and_then(|v| v.parse::<i64>().ok());
        let present = |i: usize| !matches!(chunk[i], redis::Value::Nil);
        let state = if present(1) {
            JobState::Waiting
        } else if present(2) {
            JobState::Active
        } else if present(3) {
            JobState::Paused
        } else if present(5) {
            JobState::Failed
        } else if present(6) {
            JobState::Delayed
        } else if present(4) {
            JobState::Completed
        } else if present(7) {
            JobState::Prioritized
        } else if present(8) {
            JobState::WaitingChildren
        } else {
            JobState::Unknown
        };
        let exists: bool = redis::from_redis_value(get(9))?;
        let pending: Vec<String> = redis::from_redis_value(get(10))?;
        let processed: Vec<String> = redis::from_redis_value(get(11))?;
        let ignored: Vec<String> = redis::from_redis_value(get(12))?;
        let unsuccessful: Vec<String> = redis::from_redis_value(get(13))?;
        let duration_ms = match (num(2), num(3)) {
            (Some(p), Some(f)) if f >= p => Some(f - p),
            _ => None,
        };
        let counts = (
            processed.len() as u64,
            (ignored.len() + unsuccessful.len()) as u64,
        );
        let mut children = pending;
        for key in processed.into_iter().chain(ignored).chain(unsuccessful) {
            if !children.contains(&key) {
                children.push(key);
            }
        }
        children.sort();
        probes.push(FlowProbe {
            exists,
            name: field(0).unwrap_or_default(),
            timestamp: num(1),
            duration_ms,
            failed_reason: field(4),
            parent_key: field(5),
            opts: field(6),
            return_value: field(7),
            state,
            children,
            completed: counts.0,
            failed: counts.1,
        });
    }
    Ok(probes)
}

/// The flow a job belongs to: up to the root through
/// `parentKey`, then down level by level through each parent's children
/// keys. Bounded (`flow::MAX_NODES`, `flow::MAX_ANCESTORS`); a job that is
/// in no flow comes back as a tree of one.
async fn handle_get_flow(
    state: &mut ActorState,
    args: &GetJobArgs,
    token: &CancellationToken,
) -> Result<FlowTree, RanchError> {
    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &args.queue_ref, token).await?;
    let known_prefix = args.queue_ref.prefix.clone();
    let cancelled = || RanchError::InvalidInput {
        reason: "Operation cancelled".to_string(),
    };

    let focus = JobRef {
        queue: args.queue_ref.clone(),
        id: args.job_id.clone(),
    };
    let focus_key = keys.job_hash(&focus);

    // 1. Up to the root.
    let mut root = focus.clone();
    let mut first = true;
    for _ in 0..flow::MAX_ANCESTORS {
        if token.is_cancelled() {
            return Err(cancelled());
        }
        let probe = probe_flow_jobs(&mut conn, keys, std::slice::from_ref(&root))
            .await
            .map_err(redis_to_ranch_err)?
            .pop();
        let Some(probe) = probe else { break };
        if first && !probe.exists {
            return Err(RanchError::JobNotFound {
                queue: args.queue_ref.name.clone(),
                id: args.job_id.clone(),
            });
        }
        first = false;
        match probe
            .parent_key
            .as_deref()
            .and_then(|k| flow::parse_job_key(k, &known_prefix))
        {
            Some(parent) => root = parent,
            None => break,
        }
    }

    // 2. Down, one pipeline per level.
    let root_key = keys.job_hash(&root);
    let mut nodes: Vec<FlowNode> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut level: Vec<(JobRef, Option<String>)> = vec![(root, None)];
    seen.insert(root_key.clone());
    let mut truncated = false;
    while !level.is_empty() && !truncated {
        if token.is_cancelled() {
            return Err(cancelled());
        }
        let refs: Vec<JobRef> = level.iter().map(|(r, _)| r.clone()).collect();
        let probes = probe_flow_jobs(&mut conn, keys, &refs)
            .await
            .map_err(redis_to_ranch_err)?;
        let mut next: Vec<(JobRef, Option<String>)> = Vec::new();
        for ((jref, parent_key), probe) in level.into_iter().zip(probes) {
            if nodes.len() >= flow::MAX_NODES {
                truncated = true;
                break;
            }
            let key = keys.job_hash(&jref);
            for child_key in &probe.children {
                if seen.contains(child_key) {
                    continue; // a cycle, or a child listed twice
                }
                if let Some(child) = flow::parse_job_key(child_key, &known_prefix) {
                    seen.insert(child_key.clone());
                    next.push((child, Some(key.clone())));
                }
            }
            nodes.push(FlowNode {
                key,
                name: probe.name,
                job_ref: jref,
                parent_key,
                state: probe.state,
                timestamp: probe.timestamp,
                duration_ms: probe.duration_ms,
                failed_reason: probe.failed_reason,
                missing: !probe.exists,
                return_value: probe.return_value,
                blocks_parent: probe.state == JobState::Failed
                    && flow::failure_blocks_parent(probe.opts.as_deref()),
            });
        }
        if nodes.len() + next.len() > flow::MAX_NODES {
            next.truncate(flow::MAX_NODES.saturating_sub(nodes.len()));
            truncated = true;
            // One more pass for what still fits, then stop.
            let refs: Vec<JobRef> = next.iter().map(|(r, _)| r.clone()).collect();
            let probes = probe_flow_jobs(&mut conn, keys, &refs)
                .await
                .map_err(redis_to_ranch_err)?;
            for ((jref, parent_key), probe) in next.into_iter().zip(probes) {
                nodes.push(FlowNode {
                    key: keys.job_hash(&jref),
                    name: probe.name,
                    job_ref: jref,
                    parent_key,
                    state: probe.state,
                    timestamp: probe.timestamp,
                    duration_ms: probe.duration_ms,
                    failed_reason: probe.failed_reason,
                    missing: !probe.exists,
                    return_value: probe.return_value,
                    blocks_parent: probe.state == JobState::Failed
                        && flow::failure_blocks_parent(probe.opts.as_deref()),
                });
            }
            break;
        }
        level = next;
    }

    // A blocking child only blocks a parent that waits for it.
    let waiting: std::collections::HashSet<&str> = nodes
        .iter()
        .filter(|n| n.state == JobState::WaitingChildren)
        .map(|n| n.key.as_str())
        .collect();
    let blocked_children: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            n.blocks_parent && n.parent_key.as_deref().is_some_and(|p| waiting.contains(p))
        })
        .map(|(i, _)| i)
        .collect();
    let blocked = !blocked_children.is_empty();
    for (i, node) in nodes.iter_mut().enumerate() {
        node.blocks_parent = blocked_children.contains(&i);
    }
    Ok(FlowTree {
        nodes,
        root_key,
        focus_key,
        truncated,
        blocked,
    })
}

/// Flow roots of a queue, bounded: every parent waiting
/// for children (up to `DISCOVERY_WAITING_CHILDREN`) plus the most recently
/// finished jobs of the queue, per state. A root = has children, no parent.
async fn handle_list_flow_roots(
    state: &mut ActorState,
    args: &QueueActionArgs,
    token: &CancellationToken,
) -> Result<FlowRoots, RanchError> {
    let mut conn = open_redis_for_actor(state, token).await?;
    let q = &args.queue_ref;
    let keys = resolve_keys_for_queue(state, &mut conn, q, token).await?;
    let recent = flow::DISCOVERY_RECENT_PER_STATE as i64;

    type Candidates = (Vec<String>, Vec<String>, Vec<String>, u64, u64);
    let mut candidates = redis::pipe();
    candidates
        .cmd("ZREVRANGE")
        .arg(keys.waiting_children(q))
        .arg(0)
        .arg(flow::DISCOVERY_WAITING_CHILDREN as i64 - 1)
        .cmd("ZREVRANGE")
        .arg(keys.completed(q))
        .arg(0)
        .arg(recent - 1)
        .cmd("ZREVRANGE")
        .arg(keys.failed(q))
        .arg(0)
        .arg(recent - 1)
        .cmd("ZCARD")
        .arg(keys.completed(q))
        .cmd("ZCARD")
        .arg(keys.failed(q));
    let (waiting_children, completed, failed, completed_total, failed_total): Candidates = tokio::select! {
        r = candidates.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    let capped = completed_total > recent as u64 || failed_total > recent as u64;

    let mut ids: Vec<String> = Vec::new();
    for id in waiting_children.into_iter().chain(completed).chain(failed) {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let refs: Vec<JobRef> = ids
        .into_iter()
        .map(|id| JobRef {
            queue: q.clone(),
            id,
        })
        .collect();

    let mut roots: Vec<FlowRoot> = Vec::new();
    for chunk in refs.chunks(200) {
        if token.is_cancelled() {
            return Err(RanchError::InvalidInput {
                reason: "Operation cancelled".to_string(),
            });
        }
        let probes = probe_flow_jobs(&mut conn, keys, chunk)
            .await
            .map_err(redis_to_ranch_err)?;
        for (jref, probe) in chunk.iter().zip(probes) {
            if probe.children.is_empty() || probe.parent_key.is_some() || !probe.exists {
                continue;
            }
            // What became of the children is their own state, not which of
            // the parent's side keys lists them: a child that failed without
            // `failParentOnFailure` stays in `:dependencies` — the parent
            // waits for it forever, which is exactly what this view is for.
            let child_refs: Vec<JobRef> = probe
                .children
                .iter()
                .take(flow::MAX_NODES)
                .filter_map(|k| flow::parse_job_key(k, &q.prefix))
                .collect();
            let child_probes = probe_flow_jobs(&mut conn, keys, &child_refs)
                .await
                .map_err(redis_to_ranch_err)?;
            let count =
                |wanted: JobState| child_probes.iter().filter(|c| c.state == wanted).count() as u64;
            // A processed child removed by `removeOnComplete` has no state
            // left; the parent's `processed` hash remembers it completed.
            let blocked = flow::is_blocked(
                probe.state,
                &child_probes
                    .iter()
                    .map(|c| (c.state, flow::failure_blocks_parent(c.opts.as_deref())))
                    .collect::<Vec<_>>(),
            );
            let completed = count(JobState::Completed).max(probe.completed);
            let failed = count(JobState::Failed).max(probe.failed);
            let total = probe.children.len() as u64;
            roots.push(FlowRoot {
                job_ref: jref.clone(),
                name: probe.name,
                state: probe.state,
                timestamp: probe.timestamp,
                children_total: total,
                children_pending: total.saturating_sub(completed + failed),
                children_completed: completed,
                children_failed: failed,
                blocked,
            });
        }
    }
    roots.sort_by_key(|r| std::cmp::Reverse(r.timestamp));

    Ok(FlowRoots {
        roots,
        scanned: refs.len() as u64,
        capped,
    })
}

/// Ids of the pending runs of the queue's job schedulers
/// (`repeat:{scheduler}:{next millis}`), as `drain_queue.lua` computes them.
async fn scheduler_job_ids(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
) -> Result<Vec<String>, redis::RedisError> {
    let schedulers: Vec<(String, f64)> = redis::cmd("ZRANGE")
        .arg(keys.repeat(qref))
        .arg(0)
        .arg(-1)
        .arg("WITHSCORES")
        .query_async(conn)
        .await?;
    Ok(schedulers
        .into_iter()
        .map(|(id, next)| format!("repeat:{id}:{}", next as i64))
        .collect())
}

/// What a maintenance operation would touch. Exact for
/// clean and retry; for drain an upper bound minus the scheduler runs it
/// spares (flow members are only recognised while running).
async fn handle_preview_queue_maintenance(
    state: &mut ActorState,
    args: &QueueMaintenanceArgs,
    token: &CancellationToken,
) -> Result<MaintenancePreview, RanchError> {
    let mut conn = open_redis_for_actor(state, token).await?;
    let q = &args.queue_ref;
    let keys = resolve_keys_for_queue(state, &mut conn, q, token).await?;
    let now = chrono::Utc::now().timestamp_millis();

    let preview = async {
        match &args.operation {
            QueueMaintenance::Drain { include_delayed } => {
                let (wait, paused, prioritized, delayed): (u64, u64, u64, u64) = redis::pipe()
                    .cmd("LLEN")
                    .arg(keys.wait(q))
                    .cmd("LLEN")
                    .arg(keys.paused(q))
                    .cmd("ZCARD")
                    .arg(keys.prioritized(q))
                    .cmd("ZCARD")
                    .arg(keys.delayed(q))
                    .query_async(&mut conn)
                    .await?;
                // A scheduler's pending run sits in `delayed` — or in `wait`
                // when it is due now (an `every` scheduler's first run is).
                let mut spared = 0u64;
                for id in scheduler_job_ids(&mut conn, keys, q).await? {
                    let (in_wait, in_paused, in_prioritized, in_delayed): (
                        Option<i64>,
                        Option<i64>,
                        Option<f64>,
                        Option<f64>,
                    ) = redis::pipe()
                        .cmd("LPOS")
                        .arg(keys.wait(q))
                        .arg(&id)
                        .cmd("LPOS")
                        .arg(keys.paused(q))
                        .arg(&id)
                        .cmd("ZSCORE")
                        .arg(keys.prioritized(q))
                        .arg(&id)
                        .cmd("ZSCORE")
                        .arg(keys.delayed(q))
                        .arg(&id)
                        .query_async(&mut conn)
                        .await?;
                    if in_wait.is_some()
                        || in_paused.is_some()
                        || in_prioritized.is_some()
                        || (*include_delayed && in_delayed.is_some())
                    {
                        spared += 1;
                    }
                }
                let delayed = if *include_delayed { delayed } else { 0 };
                Ok::<_, redis::RedisError>(MaintenancePreview {
                    affected: (wait + paused + prioritized + delayed).saturating_sub(spared),
                    spared_scheduler_jobs: spared,
                })
            }
            QueueMaintenance::Clean {
                state: clean_state,
                older_than_ms,
            } => {
                let set = match clean_state {
                    CleanState::Completed => keys.completed(q),
                    CleanState::Failed => keys.failed(q),
                };
                let cutoff = now - i64::try_from(*older_than_ms).unwrap_or(i64::MAX);
                let affected: u64 = redis::cmd("ZCOUNT")
                    .arg(set)
                    .arg("-inf")
                    .arg(cutoff)
                    .query_async(&mut conn)
                    .await?;
                Ok(MaintenancePreview {
                    affected,
                    spared_scheduler_jobs: 0,
                })
            }
            QueueMaintenance::RetryAllFailed => {
                let affected: u64 = redis::cmd("ZCARD")
                    .arg(keys.failed(q))
                    .query_async(&mut conn)
                    .await?;
                Ok(MaintenancePreview {
                    affected,
                    spared_scheduler_jobs: 0,
                })
            }
        }
    };
    tokio::select! {
        r = preview => r.map_err(redis_to_ranch_err),
        _ = token.cancelled() => Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    }
}

/// Drain / clean / retry all failed.
///
///   1. safety gate, once, for the whole operation;
///   2. the preview count becomes the progress total;
///   3. the operation's script runs in batches of `maintenance::BATCH`;
///      between two batches: cancellation check + a `bulk:progress` event
///      (the same event the bulk actions emit, so the same progress UI);
///   4. one audit row: what was done, what was left in place, whether it
///      was interrupted — also when a batch failed half-way.
async fn handle_run_queue_maintenance(
    state: &mut ActorState,
    args: &QueueMaintenanceArgs,
    attribution: &AuditAttribution,
    app: Option<&crate::core::events::AppHandle>,
    token: &CancellationToken,
) -> Result<MaintenanceResult, RanchError> {
    let op = &args.operation;
    authorize_write(state, attribution, op.tool()).await?;

    let total = handle_preview_queue_maintenance(state, args, token)
        .await?
        .affected;
    let mut conn = open_redis_for_actor(state, token).await?;
    let q = &args.queue_ref;
    let keys = resolve_keys_for_queue(state, &mut conn, q, token).await?;
    let prefix = format!("{}:", keys.queue_base(q));
    let batch = maintenance::BATCH;
    let now = chrono::Utc::now().timestamp_millis();

    let mut affected: u64 = 0;
    let mut skipped: u64 = 0;
    let mut cancelled = false;
    let mut error: Option<redis::RedisError> = None;

    let progress = |affected: u64, skipped: u64| {
        if let Some(a) = app {
            let clamp = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
            emit(
                a,
                "bulk:progress",
                BulkProgressEvent {
                    action: op.verb().to_string(),
                    queue_name: q.name.clone(),
                    total: clamp(total.max(affected + skipped)),
                    completed: clamp(affected + skipped),
                    failed_so_far: 0,
                },
            );
        }
    };

    // One container at a time; each loop iteration is one script call.
    // `offset` = jobs skipped so far, which stay at the head of the container.
    let containers: Vec<(String, &str)> = match op {
        QueueMaintenance::Drain { include_delayed } => {
            let mut c = vec![
                (keys.wait(q), "list"),
                (keys.paused(q), "list"),
                (keys.prioritized(q), "zset"),
            ];
            if *include_delayed {
                c.push((keys.delayed(q), "zset"));
            }
            c
        }
        QueueMaintenance::Clean {
            state: clean_state, ..
        } => vec![(
            match clean_state {
                CleanState::Completed => keys.completed(q),
                CleanState::Failed => keys.failed(q),
            },
            "zset",
        )],
        QueueMaintenance::RetryAllFailed => vec![(keys.failed(q), "zset")],
    };

    'containers: for (container, shape) in &containers {
        let mut offset: u64 = 0;
        loop {
            if token.is_cancelled() {
                cancelled = true;
                break 'containers;
            }
            let reply: Result<(i64, i64, i64), redis::RedisError> = match op {
                QueueMaintenance::Drain { .. } => {
                    redis::Script::new(&maintenance::drain_queue_lua())
                        .key(container)
                        .key(keys.repeat(q))
                        .arg(&prefix)
                        .arg(*shape)
                        .arg(offset)
                        .arg(batch)
                        .invoke_async(&mut conn)
                        .await
                }
                QueueMaintenance::Clean { older_than_ms, .. } => {
                    let cutoff = now - i64::try_from(*older_than_ms).unwrap_or(i64::MAX);
                    redis::Script::new(&maintenance::clean_jobs_lua())
                        .key(container)
                        .key(keys.events(q))
                        .arg(&prefix)
                        .arg(cutoff)
                        .arg(offset)
                        .arg(batch)
                        .invoke_async(&mut conn)
                        .await
                }
                QueueMaintenance::RetryAllFailed => {
                    redis::Script::new(&maintenance::retry_all_failed_lua())
                        .key(container)
                        .key(keys.wait(q))
                        .key(keys.paused(q))
                        .key(keys.meta(q))
                        .key(keys.active(q))
                        .key(keys.events(q))
                        .key(keys.marker(q))
                        .arg(&prefix)
                        .arg(now)
                        .arg(batch)
                        .invoke_async(&mut conn)
                        .await
                }
            };
            let reply = match reply {
                Ok(r) => BatchReply::from(r),
                Err(e) => {
                    error = Some(e);
                    break 'containers;
                }
            };
            let done = match op {
                // { retried, gone, remaining }
                QueueMaintenance::RetryAllFailed => {
                    affected += reply.first;
                    skipped += reply.second;
                    reply.third == 0 || reply.first + reply.second == 0
                }
                // { removed, skipped, scanned }
                _ => {
                    affected += reply.first;
                    skipped += reply.second;
                    offset += reply.second;
                    reply.third < batch as u64
                }
            };
            progress(affected, skipped);
            if done {
                break;
            }
        }
    }

    let (action, summary_verb) = match op {
        QueueMaintenance::Drain { .. } => (AuditAction::DrainQueue, "removed"),
        QueueMaintenance::Clean { .. } => (AuditAction::CleanQueue, "removed"),
        QueueMaintenance::RetryAllFailed => (AuditAction::RetryAllFailed, "retried"),
    };
    let mut reason = format!("{affected} {summary_verb}");
    if skipped > 0 {
        reason.push_str(&format!(", {skipped} left in place"));
    }
    if let QueueMaintenance::Clean {
        state: clean_state,
        older_than_ms,
    } = op
    {
        reason.push_str(&format!(
            " ({} older than {older_than_ms} ms)",
            match clean_state {
                CleanState::Completed => "completed",
                CleanState::Failed => "failed",
            }
        ));
    }
    let outcome = if let Some(e) = &error {
        reason.push_str(&format!(" — stopped on error: {e}"));
        AuditOutcome::Failure
    } else if cancelled {
        reason.push_str(" — cancelled");
        AuditOutcome::Cancelled
    } else {
        AuditOutcome::Success
    };
    write_audit_for_action(
        state,
        action,
        AuditTarget::Queue {
            queue_name: q.name.clone(),
        },
        outcome,
        Some(reason),
        attribution,
    )
    .await?;

    if let Some(e) = error {
        return Err(redis_to_ranch_err(e));
    }
    Ok(MaintenanceResult {
        action: op.verb().to_string(),
        affected,
        skipped,
        cancelled,
    })
}

/// Enqueue a job.
///
///   1. safety gate (MCP approval / session lock) — before anything else;
///   2. validate the input (pure, `NewJob::prepare`) — not audited, nothing
///      was attempted;
///   3. the queue must exist (`meta`): neither the UI nor an agent may
///      conjure a queue out of a typo;
///   4. one Lua script does what BullMQ's add-job scripts do — id, hash,
///      placement by delay / priority / paused, marker, events — atomically;
///   5. audit, success or failure.
async fn handle_add_job(
    state: &mut ActorState,
    args: &AddJobArgs,
    attribution: &AuditAttribution,
    token: &CancellationToken,
) -> Result<AddJobResult, RanchError> {
    authorize_write(state, attribution, "add_job").await?;

    let job = args
        .job
        .prepare()
        .map_err(|reason| RanchError::InvalidInput { reason })?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let q = &args.queue_ref;

    // Before the schema: on a cold cache a missing queue would otherwise
    // read as "schema not recognized". `:meta` is the same key in every
    // supported version.
    let mut exists_cmd = redis::cmd("EXISTS");
    exists_cmd.arg(V5Keys.meta(q));
    let queue_exists: bool = tokio::select! {
        r = exists_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    if !queue_exists {
        return Err(RanchError::InvalidInput {
            reason: format!(
                "Queue \"{}\" does not exist on this connection (prefix \"{}\")",
                q.name, q.prefix
            ),
        });
    }
    let keys = resolve_keys_for_queue(state, &mut conn, q, token).await?;

    let script = redis::Script::new(add_job::ADD_JOB_LUA);
    let mut invocation = script.prepare_invoke();
    invocation
        .key(keys.wait(q))
        .key(keys.paused(q))
        .key(keys.meta(q))
        .key(keys.id_counter(q))
        .key(keys.delayed(q))
        .key(keys.prioritized(q))
        .key(keys.active(q))
        .key(keys.events(q))
        .key(keys.marker(q))
        .key(keys.priority_counter(q))
        .arg(format!("{}:", keys.queue_base(q)))
        .arg(&job.name)
        .arg(&job.data)
        .arg(&job.opts)
        .arg(chrono::Utc::now().timestamp_millis())
        .arg(job.delay_ms)
        .arg(job.priority)
        .arg(if job.lifo { "1" } else { "0" })
        .arg(job.job_id.as_deref().unwrap_or(""));
    let result: Result<(String, String), redis::RedisError> = tokio::select! {
        r = invocation.invoke_async(&mut conn) => r,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };

    match result {
        // A custom id already in use: BullMQ would ignore the
        // add; Ranch says so instead of pretending.
        Ok((job_id, placement)) if placement == "exists" => Err(RanchError::InvalidInput {
            reason: format!(
                "A job with ID \"{job_id}\" already exists in {}: nothing was added",
                q.name
            ),
        }),
        Ok((job_id, placement)) => {
            write_audit_for_action(
                state,
                AuditAction::AddJob,
                AuditTarget::Job {
                    queue_name: q.name.clone(),
                    job_id: job_id.clone(),
                },
                AuditOutcome::Success,
                Some(format!("{} → {placement}", job.name)),
                attribution,
            )
            .await?;
            let state = match placement.as_str() {
                "delayed" => JobState::Delayed,
                "prioritized" => JobState::Prioritized,
                "paused" => JobState::Paused,
                _ => JobState::Waiting,
            };
            Ok(AddJobResult { job_id, state })
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::AddJob,
                AuditTarget::Queue {
                    queue_name: q.name.clone(),
                },
                AuditOutcome::Failure,
                Some(e.to_string()),
                attribution,
            )
            .await?;
            Err(redis_to_ranch_err(e))
        }
    }
}

/// Run `retry_jobs.lua` for `ids`: one [`MoveCode`] per id.
///
/// Replaces a plain `MULTI/EXEC`, which pushed straight into `wait`:
/// on a **paused** queue BullMQ's resume `RENAME`s `paused` over `wait`, so
/// the retried job vanished from every list; idle workers were not woken
/// (no marker); no `waiting` event; and it wrote `failedReason ""` /
/// `attemptsMade 0` — fields BullMQ does not read that way. The script does
/// what BullMQ's `reprocessJob` does.
async fn run_retry_script(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    ids: &[String],
) -> Result<Vec<MoveCode>, redis::RedisError> {
    let script = redis::Script::new(&maintenance::retry_jobs_lua());
    let mut invocation = script.prepare_invoke();
    invocation
        .key(keys.failed(qref))
        .key(keys.wait(qref))
        .key(keys.paused(qref))
        .key(keys.meta(qref))
        .key(keys.active(qref))
        .key(keys.events(qref))
        .key(keys.marker(qref))
        .arg(format!("{}:", keys.queue_base(qref)));
    for id in ids {
        invocation.arg(id);
    }
    let codes: Vec<i64> = invocation.invoke_async(conn).await?;
    Ok(codes.into_iter().map(MoveCode::from_script).collect())
}

/// Run `promote_jobs.lua` for `ids`. Same reason as
/// [`run_retry_script`]: the old `MULTI/EXEC` pushed into `wait` whatever the
/// queue's state, ignored the job's priority and woke nobody.
async fn run_promote_script(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    ids: &[String],
) -> Result<Vec<MoveCode>, redis::RedisError> {
    let script = redis::Script::new(&maintenance::promote_jobs_lua());
    let mut invocation = script.prepare_invoke();
    invocation
        .key(keys.delayed(qref))
        .key(keys.wait(qref))
        .key(keys.paused(qref))
        .key(keys.meta(qref))
        .key(keys.active(qref))
        .key(keys.events(qref))
        .key(keys.marker(qref))
        .key(keys.prioritized(qref))
        .key(keys.priority_counter(qref))
        .arg(format!("{}:", keys.queue_base(qref)));
    for id in ids {
        invocation.arg(id);
    }
    let codes: Vec<i64> = invocation.invoke_async(conn).await?;
    Ok(codes.into_iter().map(MoveCode::from_script).collect())
}

/// Retry a failed job. The write is
/// `retry_jobs.lua` (see [`run_retry_script`]): back in `wait` — or `paused`
/// — with its failure erased, workers woken, a `waiting` event emitted.
///
/// State precondition: job MUST be in `failed` state. Other states
/// surface `RanchError::InvalidJobState`. The frontend's button
/// disable is cosmetic; this check is the actual guard.
async fn handle_retry_job(
    state: &mut ActorState,
    args: &JobActionArgs,
    attribution: &AuditAttribution,
    token: &CancellationToken,
) -> Result<JobActionResult, RanchError> {
    // 1. SAFETY GATE — first line. A locked connection never touches Redis.
    authorize_write(state, attribution, "retry_job").await?;

    // 2. Open Redis + resolve keys.
    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &args.queue_ref, token).await?;

    // 3. Validate state precondition: job IS in `failed`.
    let failed_key = keys.failed(&args.queue_ref);
    let mut zscore_cmd = redis::cmd("ZSCORE");
    zscore_cmd.arg(&failed_key).arg(&args.job_id);
    let in_failed: Option<f64> = tokio::select! {
        r = zscore_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    if in_failed.is_none() {
        let current =
            detect_job_state(&mut conn, keys, &args.queue_ref, &args.job_id, token).await?;
        // Precondition failure is NOT audited (the lock sentinel covers
        // the lock-rejection case; state-precondition rejections are
        // just a UI sync issue and would noise up the audit log).
        return Err(RanchError::InvalidJobState {
            action: "retry".into(),
            connection_id: state.cfg.connection_id.clone(),
            queue_name: args.queue_ref.name.clone(),
            job_id: args.job_id.clone(),
            current_state: current,
            required_state: "failed".into(),
        });
    }

    // 4. Atomic retry — one script (see `run_retry_script`).
    let ids = [args.job_id.clone()];
    let result: Result<Vec<MoveCode>, redis::RedisError> = tokio::select! {
        r = run_retry_script(&mut conn, keys, &args.queue_ref, &ids) => r,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    // Lost a race between the check above and the script (a worker or
    // another client moved the job): same answer as the precondition.
    if let Ok(codes) = &result {
        if codes.first() != Some(&MoveCode::Moved) {
            let current =
                detect_job_state(&mut conn, keys, &args.queue_ref, &args.job_id, token).await?;
            return Err(RanchError::InvalidJobState {
                action: "retry".into(),
                connection_id: state.cfg.connection_id.clone(),
                queue_name: args.queue_ref.name.clone(),
                job_id: args.job_id.clone(),
                current_state: current,
                required_state: "failed".into(),
            });
        }
    }

    // 5. Audit (success OR failure) — last step before return.
    let target = AuditTarget::Job {
        queue_name: args.queue_ref.name.clone(),
        job_id: args.job_id.clone(),
    };
    match &result {
        Ok(_) => {
            write_audit_for_action(
                state,
                AuditAction::RetryJob,
                target,
                AuditOutcome::Success,
                None,
                attribution,
            )
            .await?;
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::RetryJob,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                attribution,
            )
            .await?;
        }
    };

    result.map_err(redis_to_ranch_err)?;
    Ok(JobActionResult {
        job_id: args.job_id.clone(),
        new_state: Some(JobState::Waiting),
        action: "retry".into(),
    })
}

/// Remove a job from every state container + the job hash + logs.
///
/// Algorithm:
///   1. EXISTS bull:{q}:jobId → JobNotFound if 0.
///   2. detect_job_state → which container holds the job (if any).
///   3. MULTI/EXEC: remove from that container (LREM or ZREM) + DEL
///      job_hash + DEL logs + DEL lock.
///   4. Audit.
async fn handle_remove_job(
    state: &mut ActorState,
    args: &JobActionArgs,
    attribution: &AuditAttribution,
    token: &CancellationToken,
) -> Result<JobActionResult, RanchError> {
    authorize_write(state, attribution, "remove_job").await?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &args.queue_ref, token).await?;

    let jref = JobRef {
        queue: args.queue_ref.clone(),
        id: args.job_id.clone(),
    };
    let job_hash_key = keys.job_hash(&jref);
    let logs_key = keys.logs(&jref);
    let lock_key = format!("{}:lock", job_hash_key);

    // 1. EXISTS guard.
    let mut exists_cmd = redis::cmd("EXISTS");
    exists_cmd.arg(&job_hash_key);
    let exists: i64 = tokio::select! {
        r = exists_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    if exists == 0 {
        return Err(RanchError::JobNotFound {
            queue: args.queue_ref.name.clone(),
            id: args.job_id.clone(),
        });
    }

    // 2. Detect the state container so we know which key to remove from.
    let detected = detect_job_state(&mut conn, keys, &args.queue_ref, &args.job_id, token).await?;

    // 3. MULTI/EXEC: container-removal + hash + logs + lock.
    let mut pipe = redis::pipe();
    pipe.atomic();
    match detected.as_str() {
        "waiting" => {
            pipe.cmd("LREM")
                .arg(keys.wait(&args.queue_ref))
                .arg(0)
                .arg(&args.job_id);
        }
        "active" => {
            pipe.cmd("LREM")
                .arg(keys.active(&args.queue_ref))
                .arg(0)
                .arg(&args.job_id);
        }
        "paused" => {
            pipe.cmd("LREM")
                .arg(keys.paused(&args.queue_ref))
                .arg(0)
                .arg(&args.job_id);
        }
        "completed" => {
            pipe.cmd("ZREM")
                .arg(keys.completed(&args.queue_ref))
                .arg(&args.job_id);
        }
        "failed" => {
            pipe.cmd("ZREM")
                .arg(keys.failed(&args.queue_ref))
                .arg(&args.job_id);
        }
        "delayed" => {
            pipe.cmd("ZREM")
                .arg(keys.delayed(&args.queue_ref))
                .arg(&args.job_id);
        }
        "prioritized" => {
            pipe.cmd("ZREM")
                .arg(keys.prioritized(&args.queue_ref))
                .arg(&args.job_id);
        }
        "waitingChildren" => {
            pipe.cmd("ZREM")
                .arg(keys.waiting_children(&args.queue_ref))
                .arg(&args.job_id);
        }
        "stalled" => {
            pipe.cmd("ZREM")
                .arg(keys.stalled(&args.queue_ref))
                .arg(&args.job_id);
        }
        _ => {
            // Hash exists but no container holds the id — very rare
            // mid-state race. Still delete the hash + logs.
            // (Add a no-op so the pipeline is non-empty.)
            pipe.cmd("PING");
        }
    }
    pipe.cmd("DEL").arg(&job_hash_key);
    pipe.cmd("DEL").arg(&logs_key);
    pipe.cmd("DEL").arg(&lock_key);

    let result: Result<redis::Value, redis::RedisError> = tokio::select! {
        r = pipe.query_async(&mut conn) => r,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };

    let target = AuditTarget::Job {
        queue_name: args.queue_ref.name.clone(),
        job_id: args.job_id.clone(),
    };
    match &result {
        Ok(_) => {
            write_audit_for_action(
                state,
                AuditAction::RemoveJob,
                target,
                AuditOutcome::Success,
                None,
                attribution,
            )
            .await?;
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::RemoveJob,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                attribution,
            )
            .await?;
        }
    };

    result.map_err(redis_to_ranch_err)?;
    Ok(JobActionResult {
        job_id: args.job_id.clone(),
        new_state: None,
        action: "remove".into(),
    })
}

/// Promote a delayed job into the waiting list.
///
/// Atomic semantics (single MULTI/EXEC):
///   - `ZREM bull:{q}:delayed jobId`
///   - `HSET bull:{q}:jobId delay 0`
///   - `LPUSH bull:{q}:wait jobId`
async fn handle_promote_job(
    state: &mut ActorState,
    args: &JobActionArgs,
    attribution: &AuditAttribution,
    token: &CancellationToken,
) -> Result<JobActionResult, RanchError> {
    authorize_write(state, attribution, "promote_job").await?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &args.queue_ref, token).await?;

    // Validate state precondition: job IS in `delayed`.
    let delayed_key = keys.delayed(&args.queue_ref);
    let mut zscore_cmd = redis::cmd("ZSCORE");
    zscore_cmd.arg(&delayed_key).arg(&args.job_id);
    let in_delayed: Option<f64> = tokio::select! {
        r = zscore_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    if in_delayed.is_none() {
        let current =
            detect_job_state(&mut conn, keys, &args.queue_ref, &args.job_id, token).await?;
        return Err(RanchError::InvalidJobState {
            action: "promote".into(),
            connection_id: state.cfg.connection_id.clone(),
            queue_name: args.queue_ref.name.clone(),
            job_id: args.job_id.clone(),
            current_state: current,
            required_state: "delayed".into(),
        });
    }

    // One script (see `run_promote_script`).
    let ids = [args.job_id.clone()];
    let result: Result<Vec<MoveCode>, redis::RedisError> = tokio::select! {
        r = run_promote_script(&mut conn, keys, &args.queue_ref, &ids) => r,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    if let Ok(codes) = &result {
        if codes.first() != Some(&MoveCode::Moved) {
            let current =
                detect_job_state(&mut conn, keys, &args.queue_ref, &args.job_id, token).await?;
            return Err(RanchError::InvalidJobState {
                action: "promote".into(),
                connection_id: state.cfg.connection_id.clone(),
                queue_name: args.queue_ref.name.clone(),
                job_id: args.job_id.clone(),
                current_state: current,
                required_state: "delayed".into(),
            });
        }
    }

    let target = AuditTarget::Job {
        queue_name: args.queue_ref.name.clone(),
        job_id: args.job_id.clone(),
    };
    match &result {
        Ok(_) => {
            write_audit_for_action(
                state,
                AuditAction::PromoteJob,
                target,
                AuditOutcome::Success,
                None,
                attribution,
            )
            .await?;
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::PromoteJob,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                attribution,
            )
            .await?;
        }
    };

    result.map_err(redis_to_ranch_err)?;
    Ok(JobActionResult {
        job_id: args.job_id.clone(),
        new_state: Some(JobState::Waiting),
        action: "promote".into(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Bulk single-queue destructive actions
// ─────────────────────────────────────────────────────────────────────────────
//
// Each bulk handler follows the prescribed shape:
//
//   1. check_lock_or_err(state.lock_state, ...)? ............... ONE safety gate
//   2. open Redis once + resolve KeyVariant once
//   3. for each job_id in args.job_ids:
//        a. check token.is_cancelled() — break if so
//        b. call the matching `*_one_for_bulk` helper (per-job MULTI/EXEC)
//        c. accumulate Ok → succeeded; Err(reason) → failed[]
//        d. every BULK_PROGRESS_CHUNK jobs OR on the last job, emit
//           `bulk:progress` via app.emit (if app handle present)
//   4. write_audit_for_action(state, action, BulkJobs{...}, outcome, reason, attribution) ... ONE consolidated audit
//   5. return BulkActionResult
//
// The lock check is ONCE at the top — the bulk run captures the lock
// state at start. Per UX spec, re-locking mid-bulk doesn't abort
// in-flight ops. Per-job errors don't abort the loop; only mid-bulk
// cancellation (token.is_cancelled() = true) or pre-loop failures
// (open_redis, resolve_keys) break out.

use crate::core::events::emit;

/// per-job retry helper used by `handle_bulk_retry_jobs`.
/// Returns `Ok(())` on success, `Err(reason)` on any failure
/// (precondition or Redis). The bulk handler collects the reason
/// into a `BulkFailure` entry.
///
/// State precondition: job MUST be in `failed`. The check is per-job
/// (not pre-flight) so the bulk can run on a mixed selection and
/// surface state-mismatch jobs as failures rather than rejecting
/// the whole call.
async fn retry_one_for_bulk(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    job_id: &str,
    token: &CancellationToken,
) -> Result<(), String> {
    let ids = [job_id.to_string()];
    let codes = tokio::select! {
        r = run_retry_script(conn, keys, qref, &ids) => r.map_err(|e| e.to_string())?,
        _ = token.cancelled() => return Err("cancelled".to_string()),
    };
    match codes.first() {
        Some(MoveCode::Moved) => Ok(()),
        Some(MoveCode::WrongState) => Err("not in failed state".to_string()),
        _ => Err("job not found".to_string()),
    }
}

/// per-job remove helper used by `handle_bulk_remove_jobs`.
/// Mirrors the single-job `handle_remove_job` Redis op sequence
/// (EXISTS check + detect-state + MULTI/EXEC).
async fn remove_one_for_bulk(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    job_id: &str,
    token: &CancellationToken,
) -> Result<(), String> {
    let jref = JobRef {
        queue: qref.clone(),
        id: job_id.to_string(),
    };
    let job_hash_key = keys.job_hash(&jref);
    let logs_key = keys.logs(&jref);
    let lock_key = format!("{}:lock", job_hash_key);

    let mut exists_cmd = redis::cmd("EXISTS");
    exists_cmd.arg(&job_hash_key);
    let exists: i64 = tokio::select! {
        r = exists_cmd.query_async(conn) => r.map_err(|e| e.to_string())?,
        _ = token.cancelled() => return Err("cancelled".to_string()),
    };
    if exists == 0 {
        return Err("job not found".to_string());
    }

    let detected = detect_job_state(conn, keys, qref, job_id, token)
        .await
        .map_err(|e| e.to_string())?;

    let mut pipe = redis::pipe();
    pipe.atomic();
    match detected.as_str() {
        "waiting" => {
            pipe.cmd("LREM").arg(keys.wait(qref)).arg(0).arg(job_id);
        }
        "active" => {
            pipe.cmd("LREM").arg(keys.active(qref)).arg(0).arg(job_id);
        }
        "paused" => {
            pipe.cmd("LREM").arg(keys.paused(qref)).arg(0).arg(job_id);
        }
        "completed" => {
            pipe.cmd("ZREM").arg(keys.completed(qref)).arg(job_id);
        }
        "failed" => {
            pipe.cmd("ZREM").arg(keys.failed(qref)).arg(job_id);
        }
        "delayed" => {
            pipe.cmd("ZREM").arg(keys.delayed(qref)).arg(job_id);
        }
        "prioritized" => {
            pipe.cmd("ZREM").arg(keys.prioritized(qref)).arg(job_id);
        }
        "waitingChildren" => {
            pipe.cmd("ZREM")
                .arg(keys.waiting_children(qref))
                .arg(job_id);
        }
        "stalled" => {
            pipe.cmd("ZREM").arg(keys.stalled(qref)).arg(job_id);
        }
        _ => {
            pipe.cmd("PING");
        }
    }
    pipe.cmd("DEL").arg(&job_hash_key);
    pipe.cmd("DEL").arg(&logs_key);
    pipe.cmd("DEL").arg(&lock_key);

    let _: redis::Value = tokio::select! {
        r = pipe.query_async(conn) => r.map_err(|e| e.to_string())?,
        _ = token.cancelled() => return Err("cancelled".to_string()),
    };
    Ok(())
}

/// per-job promote helper used by `handle_bulk_promote_jobs`.
async fn promote_one_for_bulk(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    job_id: &str,
    token: &CancellationToken,
) -> Result<(), String> {
    let ids = [job_id.to_string()];
    let codes = tokio::select! {
        r = run_promote_script(conn, keys, qref, &ids) => r.map_err(|e| e.to_string())?,
        _ = token.cancelled() => return Err("cancelled".to_string()),
    };
    match codes.first() {
        Some(MoveCode::Moved) => Ok(()),
        Some(MoveCode::WrongState) => Err("not in delayed state".to_string()),
        _ => Err("job not found".to_string()),
    }
}

/// Format the `outcome_reason` for a bulk audit row. Summarizes
/// success / fail / cancel counts and lists up to the first 5 job
/// failures by id + reason.
fn format_bulk_reason(
    succeeded: u32,
    total: u32,
    failed: &[BulkFailure],
    cancelled: bool,
) -> String {
    let mut s = if cancelled {
        format!(
            "Cancelled: {succeeded} of {total} succeeded before stop, {} failed",
            failed.len()
        )
    } else {
        format!("{succeeded} of {total} succeeded, {} failed", failed.len())
    };
    if !failed.is_empty() {
        s.push_str(": ");
        let preview: Vec<String> = failed
            .iter()
            .take(5)
            .map(|f| format!("{} ({})", f.job_id, f.reason))
            .collect();
        s.push_str(&preview.join(", "));
        if failed.len() > 5 {
            s.push_str(&format!(", … and {} more", failed.len() - 5));
        }
    }
    s
}

/// Inline the bulk-loop body. Each of the 3 bulk handlers calls this
/// macro with the matching per-job helper. We use a macro rather than
/// a generic `Fn` parameter because async-fn-pointers under HRTB
/// (`for<'a> Fn(...)`) don't elaborate lifetimes correctly in
/// rustc 1.95 — see the rustc note about "must implement Fn<(&'a)>
/// but actually implements Fn<(&'0)> for some specific lifetime".
/// Macro avoids the type-system limitation; the duplication is
/// confined to two lines per handler.
macro_rules! bulk_handler_body {
    ($state:ident, $args:ident, $attribution:ident, $app:ident, $token:ident, $tool:expr, $verb:expr, $audit_action:expr, $op_fn:ident) => {{
        // 1. ONE authorization at the top — the lock decision, or the
        // human approval, applies to the whole bulk.
        authorize_write($state, $attribution, $tool).await?;

        // 2. Open Redis once + resolve keys once.
        let mut conn = open_redis_for_actor($state, $token).await?;
        let keys = resolve_keys_for_queue($state, &mut conn, &$args.queue_ref, $token).await?;

        let total = $args.job_ids.len() as u32;
        let mut succeeded: u32 = 0;
        let mut failed: Vec<BulkFailure> = Vec::new();
        let mut cancelled = false;

        for (i, job_id) in $args.job_ids.iter().enumerate() {
            if $token.is_cancelled() {
                cancelled = true;
                break;
            }
            match $op_fn(&mut conn, keys, &$args.queue_ref, job_id, $token).await {
                Ok(()) => succeeded += 1,
                Err(reason) => failed.push(BulkFailure {
                    job_id: job_id.clone(),
                    reason,
                }),
            }
            let done_count = i + 1;
            if done_count % BULK_PROGRESS_CHUNK == 0 || done_count == $args.job_ids.len() {
                if let Some(a) = $app {
                    emit(
                        a,
                        "bulk:progress",
                        BulkProgressEvent {
                            action: $verb.to_string(),
                            queue_name: $args.queue_ref.name.clone(),
                            total,
                            completed: succeeded + failed.len() as u32,
                            failed_so_far: failed.len() as u32,
                        },
                    );
                }
            }
        }

        // 3. ONE consolidated audit row.
        let outcome = if cancelled {
            AuditOutcome::Cancelled
        } else if failed.is_empty() {
            AuditOutcome::Success
        } else {
            AuditOutcome::Failure
        };
        let reason = if cancelled || !failed.is_empty() {
            Some(format_bulk_reason(succeeded, total, &failed, cancelled))
        } else {
            None
        };
        write_audit_for_action(
            $state,
            $audit_action,
            AuditTarget::BulkJobs {
                queue_name: $args.queue_ref.name.clone(),
                job_count: total,
            },
            outcome,
            reason,
            $attribution,
        )
        .await?;

        Ok(BulkActionResult {
            action: $verb.to_string(),
            total,
            succeeded,
            failed,
            cancelled,
        })
    }};
}

async fn handle_bulk_retry_jobs(
    state: &mut ActorState,
    args: &BulkActionArgs,
    attribution: &AuditAttribution,
    app: Option<&crate::core::events::AppHandle>,
    token: &CancellationToken,
) -> Result<BulkActionResult, RanchError> {
    bulk_handler_body!(
        state,
        args,
        attribution,
        app,
        token,
        "bulk_retry_jobs",
        "retry",
        AuditAction::RetryJob,
        retry_one_for_bulk
    )
}

async fn handle_bulk_remove_jobs(
    state: &mut ActorState,
    args: &BulkActionArgs,
    attribution: &AuditAttribution,
    app: Option<&crate::core::events::AppHandle>,
    token: &CancellationToken,
) -> Result<BulkActionResult, RanchError> {
    bulk_handler_body!(
        state,
        args,
        attribution,
        app,
        token,
        "bulk_remove_jobs",
        "remove",
        AuditAction::RemoveJob,
        remove_one_for_bulk
    )
}

async fn handle_bulk_promote_jobs(
    state: &mut ActorState,
    args: &BulkActionArgs,
    attribution: &AuditAttribution,
    app: Option<&crate::core::events::AppHandle>,
    token: &CancellationToken,
) -> Result<BulkActionResult, RanchError> {
    bulk_handler_body!(
        state,
        args,
        attribution,
        app,
        token,
        "bulk_promote_jobs",
        "promote",
        AuditAction::PromoteJob,
        promote_one_for_bulk
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Pause / resume queue + scheduler
// ─────────────────────────────────────────────────────────────────────────────
//
// Pattern (shared by the destructive ops):
//   1. check_lock_or_err(state.lock_state, ...)?     ← safety gate, FIRST line
//   2. Resolve Redis + keys
//   3. Atomic Redis op (MULTI/EXEC where ≥2 keys are touched)
//   4. write_audit_for_action(state, action, target, outcome, reason, attribution)  ← LAST step
//   5. Return the resulting state
//
// Idempotency: pause-when-paused / resume-when-running is a
// successful no-op (`already_in_state: true`). The audit row captures
// it via `outcome_reason = "already paused"` / `"already running"`.
//
// BullMQ semantics (researched from BullMQ v5 source):
//   - `Queue.pause()`:  EXISTS :wait + RENAME :wait :paused + HSET :meta paused 1
//   - `Queue.resume()`: EXISTS :paused + RENAME :paused :wait + HSET :meta paused 0
//
// Scheduler pause/resume is Ranch-original (no public BullMQ API for it);
// see the per-handler comments.

/// Pause a queue. Idempotent. See module-level docs for
/// BullMQ semantics.
async fn handle_pause_queue(
    state: &mut ActorState,
    args: &QueueActionArgs,
    attribution: &AuditAttribution,
    token: &CancellationToken,
) -> Result<QueueActionResult, RanchError> {
    authorize_write(state, attribution, "pause_queue").await?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &args.queue_ref, token).await?;

    let wait_key = keys.wait(&args.queue_ref);
    let paused_key = keys.paused(&args.queue_ref);
    let meta_key = keys.meta(&args.queue_ref);

    // 1. EXISTS :wait outside the pipeline (MULTI/EXEC doesn't allow
    // conditional logic; we need the value to decide whether to
    // include RENAME).
    let mut exists_cmd = redis::cmd("EXISTS");
    exists_cmd.arg(&wait_key);
    let wait_exists: i64 = tokio::select! {
        r = exists_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };

    // 2. Read the current `paused` flag from :meta — if "1", the
    // call is a no-op (the user/agent re-asked for pause on an
    // already-paused queue). Still HSET the flag below to be defensive.
    let mut hget_cmd = redis::cmd("HGET");
    hget_cmd.arg(&meta_key).arg("paused");
    let current_paused: Option<String> = tokio::select! {
        r = hget_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    let already_in_state = current_paused.as_deref() == Some("1");

    // 3. MULTI/EXEC: conditionally RENAME + always HSET.
    let mut pipe = redis::pipe();
    pipe.atomic();
    if wait_exists == 1 {
        pipe.cmd("RENAME").arg(&wait_key).arg(&paused_key);
    }
    pipe.cmd("HSET").arg(&meta_key).arg("paused").arg("1");
    let result: Result<redis::Value, redis::RedisError> = tokio::select! {
        r = pipe.query_async(&mut conn) => r,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };

    // 4. Audit. Idempotent no-op gets a descriptive outcome_reason.
    let target = AuditTarget::Queue {
        queue_name: args.queue_ref.name.clone(),
    };
    match &result {
        Ok(_) => {
            let reason = if already_in_state {
                Some("already paused".to_string())
            } else {
                None
            };
            write_audit_for_action(
                state,
                AuditAction::PauseQueue,
                target,
                AuditOutcome::Success,
                reason,
                attribution,
            )
            .await?;
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::PauseQueue,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                attribution,
            )
            .await?;
        }
    };

    result.map_err(redis_to_ranch_err)?;
    Ok(QueueActionResult {
        is_paused: true,
        already_in_state,
    })
}

/// Resume a queue. Idempotent. Symmetric to `handle_pause_queue`.
async fn handle_resume_queue(
    state: &mut ActorState,
    args: &QueueActionArgs,
    attribution: &AuditAttribution,
    token: &CancellationToken,
) -> Result<QueueActionResult, RanchError> {
    authorize_write(state, attribution, "resume_queue").await?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &args.queue_ref, token).await?;

    let wait_key = keys.wait(&args.queue_ref);
    let paused_key = keys.paused(&args.queue_ref);
    let meta_key = keys.meta(&args.queue_ref);

    let mut exists_cmd = redis::cmd("EXISTS");
    exists_cmd.arg(&paused_key);
    let paused_exists: i64 = tokio::select! {
        r = exists_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };

    let mut hget_cmd = redis::cmd("HGET");
    hget_cmd.arg(&meta_key).arg("paused");
    let current_paused: Option<String> = tokio::select! {
        r = hget_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    // Already resumed means: meta.paused is not "1" (could be "0",
    // missing, or anything else). Resume should be idempotent.
    let already_in_state = current_paused.as_deref() != Some("1");

    let mut pipe = redis::pipe();
    pipe.atomic();
    if paused_exists == 1 {
        pipe.cmd("RENAME").arg(&paused_key).arg(&wait_key);
    }
    pipe.cmd("HSET").arg(&meta_key).arg("paused").arg("0");
    let result: Result<redis::Value, redis::RedisError> = tokio::select! {
        r = pipe.query_async(&mut conn) => r,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };

    let target = AuditTarget::Queue {
        queue_name: args.queue_ref.name.clone(),
    };
    match &result {
        Ok(_) => {
            let reason = if already_in_state {
                Some("already running".to_string())
            } else {
                None
            };
            write_audit_for_action(
                state,
                AuditAction::ResumeQueue,
                target,
                AuditOutcome::Success,
                reason,
                attribution,
            )
            .await?;
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::ResumeQueue,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                attribution,
            )
            .await?;
        }
    };

    result.map_err(redis_to_ranch_err)?;
    Ok(QueueActionResult {
        is_paused: false,
        already_in_state,
    })
}

/// The scheduler's settings, read back from its hash.
async fn read_scheduler_spec(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    scheduler_key: &str,
) -> Result<Option<(SchedulerSpec, bool)>, redis::RedisError> {
    let hash_key = format!("{}:{}", keys.repeat(qref), scheduler_key);
    let fields: HashMap<String, String> = redis::cmd("HGETALL")
        .arg(&hash_key)
        .query_async(conn)
        .await?;
    let Some(name) = fields.get("name") else {
        return Ok(None);
    };
    let schedule = match (fields.get("pattern"), fields.get("every")) {
        (Some(pattern), _) => Schedule::Pattern {
            pattern: pattern.clone(),
            tz: fields.get("tz").cloned(),
        },
        (None, Some(every)) => Schedule::Every {
            every_ms: every.parse().unwrap_or(0),
        },
        (None, None) => return Ok(None),
    };
    let paused = fields.get("paused").map(String::as_str) == Some("1");
    Ok(Some((
        SchedulerSpec {
            id: scheduler_key.to_string(),
            job_name: name.clone(),
            schedule,
            data: fields.get("data").cloned().unwrap_or_else(|| "{}".into()),
        },
        paused,
    )))
}

/// Run `upsert_scheduler.lua`.
async fn run_upsert_scheduler_script(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    prepared: &scheduler::PreparedScheduler,
    now_ms: i64,
) -> Result<Result<SchedulerSaved, String>, redis::RedisError> {
    let opt = |v: Option<String>| v.unwrap_or_default();
    let (job_id, second): (String, i64) = redis::Script::new(&scheduler::upsert_scheduler_lua())
        .key(keys.repeat(qref))
        .key(keys.delayed(qref))
        .key(keys.wait(qref))
        .key(keys.paused(qref))
        .key(keys.meta(qref))
        .key(keys.prioritized(qref))
        .key(keys.marker(qref))
        .key(keys.id_counter(qref))
        .key(keys.events(qref))
        .key(keys.priority_counter(qref))
        .key(keys.active(qref))
        .arg(format!("{}:", keys.queue_base(qref)))
        .arg(&prepared.id)
        .arg(&prepared.job_name)
        .arg(opt(prepared.pattern.clone()))
        .arg(opt(prepared.tz.clone()))
        .arg(opt(prepared.every_ms.map(|e| e.to_string())))
        .arg(opt(prepared.first_run_ms.map(|m| m.to_string())))
        .arg(now_ms)
        .arg(&prepared.data)
        .invoke_async(conn)
        .await?;
    if job_id.is_empty() {
        return Ok(Err(match second {
            -10 => "A job already exists for the next run of this pattern (it is running or has just finished). Try again in a moment.".to_string(),
            -11 => "Every upcoming slot of this interval already has a job. Remove the stale `repeat:` jobs of this scheduler first.".to_string(),
            code => format!("The scheduler script refused the change (code {code})"),
        }));
    }
    Ok(Ok(SchedulerSaved {
        scheduler_id: prepared.id.clone(),
        next_run_ms: now_ms + second,
        job_id,
    }))
}

/// Create or replace a job scheduler: gate → validate
/// (pure; the first run of a pattern is computed here, in the scheduler's
/// timezone) → queue must exist → `upsert_scheduler.lua` → audit.
async fn handle_upsert_scheduler(
    state: &mut ActorState,
    args: &UpsertSchedulerArgs,
    attribution: &AuditAttribution,
    token: &CancellationToken,
) -> Result<SchedulerSaved, RanchError> {
    authorize_write(state, attribution, "upsert_scheduler").await?;

    let now = chrono::Utc::now();
    let prepared = args
        .scheduler
        .prepare(now)
        .map_err(|reason| RanchError::InvalidInput { reason })?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let q = &args.queue_ref;
    let mut exists_cmd = redis::cmd("EXISTS");
    exists_cmd.arg(V5Keys.meta(q));
    let queue_exists: bool = tokio::select! {
        r = exists_cmd.query_async(&mut conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    if !queue_exists {
        return Err(RanchError::InvalidInput {
            reason: format!(
                "Queue \"{}\" does not exist on this connection (prefix \"{}\")",
                q.name, q.prefix
            ),
        });
    }
    let keys = resolve_keys_for_queue(state, &mut conn, q, token).await?;

    let result = tokio::select! {
        r = run_upsert_scheduler_script(&mut conn, keys, q, &prepared, now.timestamp_millis()) => r,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    let target = AuditTarget::Scheduler {
        queue_name: q.name.clone(),
        scheduler_key: prepared.id.clone(),
    };
    let described = match (&prepared.pattern, prepared.every_ms) {
        (Some(p), _) => format!(
            "{} · {p}{}",
            prepared.job_name,
            prepared
                .tz
                .as_deref()
                .map(|tz| format!(" ({tz})"))
                .unwrap_or_default()
        ),
        (None, Some(every)) => format!("{} · every {every} ms", prepared.job_name),
        (None, None) => prepared.job_name.clone(),
    };
    match result {
        Ok(Ok(saved)) => {
            write_audit_for_action(
                state,
                AuditAction::UpsertScheduler,
                target,
                AuditOutcome::Success,
                Some(described),
                attribution,
            )
            .await?;
            Ok(saved)
        }
        Ok(Err(refusal)) => {
            write_audit_for_action(
                state,
                AuditAction::UpsertScheduler,
                target,
                AuditOutcome::Failure,
                Some(refusal.clone()),
                attribution,
            )
            .await?;
            Err(RanchError::InvalidInput { reason: refusal })
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::UpsertScheduler,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                attribution,
            )
            .await?;
            Err(redis_to_ranch_err(e))
        }
    }
}

/// `remove_scheduler.lua`: `mode` = "remove" | "pause".
/// Returns `None` when the scheduler does not exist.
async fn run_remove_scheduler_script(
    conn: &mut RedisConn,
    keys: KeyVariant,
    qref: &QueueRef,
    scheduler_key: &str,
    mode: &str,
) -> Result<Option<bool>, redis::RedisError> {
    let (code, removed_pending): (i64, i64) =
        redis::Script::new(&scheduler::remove_scheduler_lua())
            .key(keys.repeat(qref))
            .key(keys.delayed(qref))
            .key(keys.events(qref))
            .key(keys.wait(qref))
            .key(keys.paused(qref))
            .key(keys.prioritized(qref))
            .arg(scheduler_key)
            .arg(format!("{}:", keys.queue_base(qref)))
            .arg(mode)
            .invoke_async(conn)
            .await?;
    Ok((code == 0).then_some(removed_pending == 1))
}

/// Delete a job scheduler and its pending job. A job of
/// it that is running finishes; it has no successor.
async fn handle_remove_scheduler(
    state: &mut ActorState,
    args: &SchedulerActionArgs,
    attribution: &AuditAttribution,
    token: &CancellationToken,
) -> Result<SchedulerActionResult, RanchError> {
    authorize_write(state, attribution, "remove_scheduler").await?;
    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &args.queue_ref, token).await?;

    let result = tokio::select! {
        r = run_remove_scheduler_script(&mut conn, keys, &args.queue_ref, &args.scheduler_key, "remove") => r,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    let target = AuditTarget::Scheduler {
        queue_name: args.queue_ref.name.clone(),
        scheduler_key: args.scheduler_key.clone(),
    };
    match result {
        // Not audited: nothing was attempted (same convention as the other destructive ops).
        Ok(None) => Err(RanchError::SchedulerNotFound {
            connection_id: state.cfg.connection_id.clone(),
            queue_name: args.queue_ref.name.clone(),
            scheduler_key: args.scheduler_key.clone(),
        }),
        Ok(Some(removed_pending)) => {
            write_audit_for_action(
                state,
                AuditAction::RemoveScheduler,
                target,
                AuditOutcome::Success,
                Some(if removed_pending {
                    "pending job removed".to_string()
                } else {
                    "no pending job (running or already gone)".to_string()
                }),
                attribution,
            )
            .await?;
            Ok(SchedulerActionResult {
                paused: false,
                already_in_state: false,
            })
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::RemoveScheduler,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                attribution,
            )
            .await?;
            Err(redis_to_ranch_err(e))
        }
    }
}

/// Pause a job scheduler.
///
/// BullMQ has no such notion, and no spawner either: the worker that
/// finishes a scheduler's pending job creates the next one. So pausing =
/// **removing the pending job** (`remove_scheduler.lua`, mode `pause`) and
/// flagging the hash `paused = 1`. The scheduler stays in `repeat`, hence
/// stays listed — by Ranch and by BullMQ.
///
/// (Removing the scheduler from `repeat` instead would make it vanish from
/// Ranch's own list — which reads `repeat` — so it could not be resumed from
/// the UI, and its pending job would still run once.)
async fn handle_pause_scheduler(
    state: &mut ActorState,
    args: &SchedulerActionArgs,
    attribution: &AuditAttribution,
    token: &CancellationToken,
) -> Result<SchedulerActionResult, RanchError> {
    authorize_write(state, attribution, "pause_scheduler").await?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &args.queue_ref, token).await?;
    let not_found = |state: &ActorState| RanchError::SchedulerNotFound {
        connection_id: state.cfg.connection_id.clone(),
        queue_name: args.queue_ref.name.clone(),
        scheduler_key: args.scheduler_key.clone(),
    };

    let spec = tokio::select! {
        r = read_scheduler_spec(&mut conn, keys, &args.queue_ref, &args.scheduler_key) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    let Some((_, already_paused)) = spec else {
        return Err(not_found(state));
    };

    let target = AuditTarget::Scheduler {
        queue_name: args.queue_ref.name.clone(),
        scheduler_key: args.scheduler_key.clone(),
    };
    if already_paused {
        write_audit_for_action(
            state,
            AuditAction::PauseScheduler,
            target,
            AuditOutcome::Success,
            Some("already paused".to_string()),
            attribution,
        )
        .await?;
        return Ok(SchedulerActionResult {
            paused: true,
            already_in_state: true,
        });
    }

    let result = tokio::select! {
        r = run_remove_scheduler_script(&mut conn, keys, &args.queue_ref, &args.scheduler_key, "pause") => r,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    match result {
        Ok(None) => Err(not_found(state)),
        Ok(Some(_)) => {
            write_audit_for_action(
                state,
                AuditAction::PauseScheduler,
                target,
                AuditOutcome::Success,
                None,
                attribution,
            )
            .await?;
            Ok(SchedulerActionResult {
                paused: true,
                already_in_state: false,
            })
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::PauseScheduler,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                attribution,
            )
            .await?;
            Err(redis_to_ranch_err(e))
        }
    }
}

/// Resume a paused scheduler: an upsert
/// with the settings stored in its hash, which computes the next run and
/// creates the pending job again (and drops the `paused` flag: the script
/// rewrites the hash). The iteration count is kept.
///
/// (Re-adding the scheduler to `repeat` with `now` as score would create no
/// job, and only a finished job creates the next one: the scheduler would
/// never fire again.)
async fn handle_resume_scheduler(
    state: &mut ActorState,
    args: &SchedulerActionArgs,
    attribution: &AuditAttribution,
    token: &CancellationToken,
) -> Result<SchedulerActionResult, RanchError> {
    authorize_write(state, attribution, "resume_scheduler").await?;

    let mut conn = open_redis_for_actor(state, token).await?;
    let keys = resolve_keys_for_queue(state, &mut conn, &args.queue_ref, token).await?;

    let spec = tokio::select! {
        r = read_scheduler_spec(&mut conn, keys, &args.queue_ref, &args.scheduler_key) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    let Some((spec, paused)) = spec else {
        return Err(RanchError::SchedulerNotFound {
            connection_id: state.cfg.connection_id.clone(),
            queue_name: args.queue_ref.name.clone(),
            scheduler_key: args.scheduler_key.clone(),
        });
    };
    let target = AuditTarget::Scheduler {
        queue_name: args.queue_ref.name.clone(),
        scheduler_key: args.scheduler_key.clone(),
    };
    if !paused {
        write_audit_for_action(
            state,
            AuditAction::ResumeScheduler,
            target,
            AuditOutcome::Success,
            Some("already active".to_string()),
            attribution,
        )
        .await?;
        return Ok(SchedulerActionResult {
            paused: false,
            already_in_state: true,
        });
    }

    let now = chrono::Utc::now();
    // Settings BullMQ wrote may use cron syntax Ranch does not author (`L`,
    // `#`…): refusing is better than re-arming at a time Ranch guessed.
    let prepared = spec
        .prepare(now)
        .map_err(|reason| RanchError::InvalidInput {
            reason: format!("Cannot resume this scheduler from Ranch: {reason}"),
        })?;
    let result = tokio::select! {
        r = run_upsert_scheduler_script(&mut conn, keys, &args.queue_ref, &prepared, now.timestamp_millis()) => r,
        _ = token.cancelled() => return Err(RanchError::InvalidInput {
            reason: "Operation cancelled".to_string(),
        }),
    };
    match result {
        Ok(Ok(_)) => {
            write_audit_for_action(
                state,
                AuditAction::ResumeScheduler,
                target,
                AuditOutcome::Success,
                None,
                attribution,
            )
            .await?;
            Ok(SchedulerActionResult {
                paused: false,
                already_in_state: false,
            })
        }
        Ok(Err(refusal)) => {
            write_audit_for_action(
                state,
                AuditAction::ResumeScheduler,
                target,
                AuditOutcome::Failure,
                Some(refusal.clone()),
                attribution,
            )
            .await?;
            Err(RanchError::InvalidInput { reason: refusal })
        }
        Err(e) => {
            write_audit_for_action(
                state,
                AuditAction::ResumeScheduler,
                target,
                AuditOutcome::Failure,
                Some(e.to_string()),
                attribution,
            )
            .await?;
            Err(redis_to_ranch_err(e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_queue_meta_key ────────────────────────────────────────────

    #[test]
    fn parse_default_prefix() {
        assert_eq!(
            parse_queue_meta_key("bull:emails:meta"),
            Some(("bull".to_string(), "emails".to_string()))
        );
    }

    #[test]
    fn parse_custom_prefix() {
        assert_eq!(
            parse_queue_meta_key("myapp:queues:billing:meta"),
            Some(("myapp:queues".to_string(), "billing".to_string()))
        );
    }

    #[test]
    fn parse_rejects_no_meta_suffix() {
        assert_eq!(parse_queue_meta_key("bull:emails"), None);
    }

    #[test]
    fn parse_rejects_only_meta() {
        assert_eq!(parse_queue_meta_key(":meta"), None);
        assert_eq!(parse_queue_meta_key("meta"), None);
    }

    #[test]
    fn parse_rejects_no_name_segment() {
        // "bull:meta" → strip ":meta" → "bull" → no `:` → None.
        assert_eq!(parse_queue_meta_key("bull:meta"), None);
    }

    #[test]
    fn parse_rejects_empty_prefix() {
        // ":emails:meta" → strip ":meta" → ":emails" → last `:` at idx 0 →
        // prefix "" empty → None.
        assert_eq!(parse_queue_meta_key(":emails:meta"), None);
    }

    // ── pick_or_cache_schema ────────────────────────────────────────────

    fn qref(prefix: &str, name: &str) -> QueueRef {
        QueueRef {
            prefix: prefix.into(),
            name: name.into(),
        }
    }

    #[test]
    fn pick_uses_cached_when_present() {
        let mut cached = Some(BullmqSchema::V5);
        let detected = vec![(qref("bull", "x"), BullmqSchema::V6)]; // ignored
        let s = pick_or_cache_schema(&mut cached, &detected).unwrap();
        assert_eq!(s, BullmqSchema::V5);
    }

    #[test]
    fn pick_caches_first_known_schema() {
        let mut cached: Option<BullmqSchema> = None;
        let detected = vec![
            (
                qref("bull", "a"),
                BullmqSchema::Unknown {
                    detected_version: Some("4.x".into()),
                },
            ),
            (qref("bull", "b"), BullmqSchema::V6),
            (qref("bull", "c"), BullmqSchema::V5),
        ];
        let s = pick_or_cache_schema(&mut cached, &detected).unwrap();
        assert_eq!(s, BullmqSchema::V6);
        assert_eq!(cached, Some(BullmqSchema::V6));
    }

    #[test]
    fn pick_returns_schema_not_recognized_when_all_unknown() {
        let mut cached: Option<BullmqSchema> = None;
        let detected = vec![
            (
                qref("bull", "a"),
                BullmqSchema::Unknown {
                    detected_version: Some("7.0.0".into()),
                },
            ),
            (
                qref("bull", "b"),
                BullmqSchema::Unknown {
                    detected_version: Some("8.x".into()),
                },
            ),
        ];
        let err = pick_or_cache_schema(&mut cached, &detected).unwrap_err();
        match err {
            RanchError::SchemaNotRecognized { detected_version } => {
                assert_eq!(detected_version.as_deref(), Some("7.0.0"));
            }
            other => panic!("expected SchemaNotRecognized, got {other:?}"),
        }
    }

    #[test]
    fn pick_re_attempts_when_cached_is_unknown() {
        let mut cached = Some(BullmqSchema::Unknown {
            detected_version: Some("?".into()),
        });
        let detected = vec![(qref("bull", "a"), BullmqSchema::V5)];
        let s = pick_or_cache_schema(&mut cached, &detected).unwrap();
        assert_eq!(s, BullmqSchema::V5);
        assert_eq!(cached, Some(BullmqSchema::V5));
    }
}
