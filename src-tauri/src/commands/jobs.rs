//! Job commands.
//!
//! All per-state job IO routes through `core::connection_actor`. This
//! file is the thin command-layer wrapper: resolves the connection's
//! config (SQLite + keychain) via the shared `resolve_connection_config`
//! helper, dispatches to `ConnectionRegistry::*`, plumbs the result
//! back through specta's `Result<_, RanchErrorDto>` boundary.
//!
//! ## Listing
//!
//! - `list_jobs({ connectionId, queueRef, state, offset, limit, newestFirst })`
//!   → `ListJobsOutput { jobs, total, state, hasMore }` — paginated by 50. The actor handles SCAN-free per-state membership reads
//!   (LRANGE for list states, ZRANGE/ZREVRANGE for sorted-set states),
//!   filters BullMQ v6 markers, and decodes via `core::bullmq_schema::JobDecoder`.
//!
//! ## Single job and logs
//!
//! - `get_job({ connectionId, queueRef, jobId })` → `Job` — single
//!   HGETALL + decode for the drill-down route. Surfaces
//!   `RanchError::JobNotFound` when the hash is empty (job deleted
//!   between the list and the drill-down click).
//! - `get_job_logs({ connectionId, queueRef, jobId, offset, limit })`
//!   → `JobLogsOutput { lines, total, hasMore }` — LLEN + LRANGE on
//!   the `bull:{q}:{id}:logs` BullMQ list key, paginated.

use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::SqlitePool;
use std::sync::Arc;

use crate::app_state::AppState;
use crate::commands::connections::resolve_connection_config;
use crate::core::bullmq_schema::{
    FailedJobsPage, FailureCauses, FlowTree, Job, JobSort, JobState, NewJob, QueueRef, RecentJobs,
};
use crate::core::connection_actor::ConnectionRegistry;
use crate::core::errors::{RanchError, RanchErrorDto};
use crate::core::keychain::KeychainAdapter;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListJobsArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
    pub state: JobState,
    /// 0-based offset into the state's list/sorted-set.
    pub offset: u64,
    /// Page size. The frontend uses 50 (target ≤100ms per page).
    pub limit: u64,
    /// For sorted-set states (Completed / Failed / Delayed / Prioritized),
    /// `true` returns highest-score-first (typical "newest first" UX —
    /// implemented via `ZREVRANGE`). List states (Waiting / Active /
    /// Paused) ignore this flag.
    pub newest_first: bool,
    /// case-insensitive substring of the job **id or name**,
    /// matched across the whole state (bounded scan), not the loaded page.
    #[serde(default)]
    pub search: Option<String>,
    /// case-insensitive substring of the failure reason.
    #[serde(default)]
    pub reason: Option<String>,
    /// Custom sort. `None` keeps the state's natural order.
    #[serde(default)]
    pub sort: Option<JobSort>,
}

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListJobsOutput {
    pub jobs: Vec<Job>,
    /// Jobs in the state — or, with a search / reason filter, jobs matching it.
    pub total: u64,
    pub state: JobState,
    pub has_more: bool,
    /// Jobs a search / sort looked at; `None` on the plain
    /// range-read path (nothing is scanned there).
    pub scanned: Option<u64>,
    /// The state holds more jobs than a query scans
    /// (`job_query::SCAN_CAP`): older jobs were not considered.
    pub scan_capped: bool,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_jobs(
    args: ListJobsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<ListJobsOutput, RanchErrorDto> {
    list_jobs_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn list_jobs_inner(
    args: ListJobsArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<ListJobsOutput, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.list_jobs(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// get_job + get_job_logs
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GetJobArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GetJobLogsArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
    pub job_id: String,
    /// 0-based offset into the logs list. Default 0.
    pub offset: u64,
    /// Page size. The frontend uses 200 by default — that covers
    /// the realistic case; pathological multi-thousand-line cases get
    /// pagination via Previous/Next.
    pub limit: u64,
}

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JobLogsOutput {
    pub lines: Vec<String>,
    pub total: u64,
    pub has_more: bool,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_job(
    args: GetJobArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Job, RanchErrorDto> {
    get_job_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn get_job_inner(
    args: GetJobArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<Job, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.get_job(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_job_logs(
    args: GetJobLogsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<JobLogsOutput, RanchErrorDto> {
    get_job_logs_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn get_job_logs_inner(
    args: GetJobLogsArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<JobLogsOutput, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.get_job_logs(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// single-job destructive actions (retry / remove / promote)
// ─────────────────────────────────────────────────────────────────────────────

/// Shared input shape for all three single-job actions. The action
/// kind is encoded in the Tauri command name (`retry_job` /
/// `remove_job` / `promote_job`); the body itself is identical.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JobActionArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
    pub job_id: String,
}

/// Reply for all three single-job actions. `new_state` is the
/// resulting BullMQ state on success (`waiting` for retry/promote,
/// `None` for remove since the job no longer exists). `action` is
/// the verb a frontend can use in success toasts ("retry" /
/// "remove" / "promote") without re-deriving from the command name.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JobActionResult {
    pub job_id: String,
    pub new_state: Option<JobState>,
    pub action: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Flows
// ─────────────────────────────────────────────────────────────────────────────

/// The flow (parent / child tree) a job belongs to. A job
/// in no flow comes back as a tree of one node.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_flow(
    args: GetJobArgs,
    state: tauri::State<'_, AppState>,
) -> Result<FlowTree, RanchErrorDto> {
    get_flow_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn get_flow_inner(
    args: GetJobArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<FlowTree, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.get_flow(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// add_job
// ─────────────────────────────────────────────────────────────────────────────

/// Args for `add_job`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddJobArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
    pub job: NewJob,
}

/// The job Ranch created. `state` is where BullMQ's rules put it: `waiting`,
/// `paused` (queue paused), `delayed` or `prioritized`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AddJobResult {
    pub job_id: String,
    pub state: JobState,
}

/// Enqueue a job. Lock-aware, audited; the write itself
/// is one atomic Lua script (`bullmq_schema::add_job`).
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn add_job(
    args: AddJobArgs,
    state: tauri::State<'_, AppState>,
) -> Result<AddJobResult, RanchErrorDto> {
    add_job_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn add_job_inner(
    args: AddJobArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<AddJobResult, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry
        .add_job(
            &id,
            &config,
            args,
            crate::core::connection_actor::AuditAttribution::ui(),
        )
        .await
    {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn retry_job(
    args: JobActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<JobActionResult, RanchErrorDto> {
    retry_job_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn retry_job_inner(
    args: JobActionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<JobActionResult, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry
        .retry_job(
            &id,
            &config,
            args,
            crate::core::connection_actor::AuditAttribution::ui(),
        )
        .await
    {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn remove_job(
    args: JobActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<JobActionResult, RanchErrorDto> {
    remove_job_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn remove_job_inner(
    args: JobActionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<JobActionResult, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry
        .remove_job(
            &id,
            &config,
            args,
            crate::core::connection_actor::AuditAttribution::ui(),
        )
        .await
    {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn promote_job(
    args: JobActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<JobActionResult, RanchErrorDto> {
    promote_job_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn promote_job_inner(
    args: JobActionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<JobActionResult, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry
        .promote_job(
            &id,
            &config,
            args,
            crate::core::connection_actor::AuditAttribution::ui(),
        )
        .await
    {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Bulk single-queue destructive actions
// ─────────────────────────────────────────────────────────────────────────────

/// Input for a bulk action over a list of jobs within a
/// single queue; cross-queue scope is not supported.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BulkActionArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
    pub job_ids: Vec<String>,
}

/// per-job failure entry collected during a bulk run.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BulkFailure {
    pub job_id: String,
    pub reason: String,
}

/// Outcome of a bulk action. The frontend's
/// `BulkActionBar` renders the summary from these fields. `cancelled`
/// is true when the actor's cancellation token fired mid-loop
/// (frontend-abandons-promise path = stays false; only true if the
/// actor is shutdown).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BulkActionResult {
    pub action: String,
    pub total: u32,
    pub succeeded: u32,
    pub failed: Vec<BulkFailure>,
    pub cancelled: bool,
}

/// Typed progress event emitted by the actor every N
/// completed jobs (see `BULK_PROGRESS_CHUNK` for chunk size). The
/// frontend subscribes via `@tauri-apps/api/event::listen("bulk:progress")`
/// and drives the progress bar live.
///
/// `queue_name` added so the frontend can disambiguate
/// concurrent per-queue progress streams during a cross-queue bulk
/// run. Listeners that don't consume the field keep working.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BulkProgressEvent {
    pub action: String,
    pub queue_name: String,
    pub total: u32,
    pub completed: u32,
    pub failed_so_far: u32,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn bulk_retry_jobs(
    args: BulkActionArgs,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<BulkActionResult, RanchErrorDto> {
    bulk_retry_jobs_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
        Some(&app),
    )
    .await
    .map_err(Into::into)
}

pub async fn bulk_retry_jobs_inner(
    args: BulkActionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
    app: Option<&crate::core::events::AppHandle>,
) -> Result<BulkActionResult, RanchError> {
    bulk_dispatch(args, pool, keychain, registry, app, BulkActionKind::Retry).await
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn bulk_remove_jobs(
    args: BulkActionArgs,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<BulkActionResult, RanchErrorDto> {
    bulk_remove_jobs_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
        Some(&app),
    )
    .await
    .map_err(Into::into)
}

pub async fn bulk_remove_jobs_inner(
    args: BulkActionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
    app: Option<&crate::core::events::AppHandle>,
) -> Result<BulkActionResult, RanchError> {
    bulk_dispatch(args, pool, keychain, registry, app, BulkActionKind::Remove).await
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn bulk_promote_jobs(
    args: BulkActionArgs,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<BulkActionResult, RanchErrorDto> {
    bulk_promote_jobs_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
        Some(&app),
    )
    .await
    .map_err(Into::into)
}

pub async fn bulk_promote_jobs_inner(
    args: BulkActionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
    app: Option<&crate::core::events::AppHandle>,
) -> Result<BulkActionResult, RanchError> {
    bulk_dispatch(args, pool, keychain, registry, app, BulkActionKind::Promote).await
}

/// Internal enum used to keep the three bulk Tauri commands DRY. The
/// per-kind dispatch is one match arm in `bulk_dispatch`.
#[derive(Debug, Clone, Copy)]
pub enum BulkActionKind {
    Retry,
    Remove,
    Promote,
}

impl BulkActionKind {
    pub fn verb(self) -> &'static str {
        match self {
            Self::Retry => "retry",
            Self::Remove => "remove",
            Self::Promote => "promote",
        }
    }

    pub fn audit_action(self) -> crate::core::audit::AuditAction {
        use crate::core::audit::AuditAction;
        match self {
            Self::Retry => AuditAction::RetryJob,
            Self::Remove => AuditAction::RemoveJob,
            Self::Promote => AuditAction::PromoteJob,
        }
    }
}

/// Shared dispatch path for all three bulk commands. Resolves the
/// connection config + dispatches to the registry + handles the
/// consolidated lock-rejection audit.
async fn bulk_dispatch(
    args: BulkActionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
    app: Option<&crate::core::events::AppHandle>,
    kind: BulkActionKind,
) -> Result<BulkActionResult, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    let job_count = args.job_ids.len() as u32;
    let queue_name = args.queue_ref.name.clone();
    let outcome_result = match kind {
        BulkActionKind::Retry => registry.bulk_retry_jobs(&id, &config, args, app).await,
        BulkActionKind::Remove => registry.bulk_remove_jobs(&id, &config, args, app).await,
        BulkActionKind::Promote => registry.bulk_promote_jobs(&id, &config, args, app).await,
    };

    match outcome_result {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(RanchError::ConnectionLocked {
            connection_id,
            connection_name,
            unlock_hint,
        })) => {
            // Consolidated bulk-lock audit row.
            // Best-effort: failure to write the audit row should NOT
            // mask the original ConnectionLocked error from the user.
            let entry = crate::core::audit::AuditEntry {
                source: crate::core::audit::AuditSource::Ui,
                action: kind.audit_action(),
                target: crate::core::audit::AuditTarget::BulkJobs {
                    queue_name,
                    job_count,
                },
                outcome: crate::core::audit::AuditOutcome::Failure,
                outcome_reason: Some("Connection locked".to_string()),
                connection_id: Some(connection_id.clone()),
                connection_name: Some(connection_name.clone()),
                env_color: config.env_color.clone(),
                agent_client: None,
                agent_transport: None,
                approval_id: None,
            };
            if let Err(e) = crate::core::audit::record(pool, entry).await {
                tracing::warn!(error = %e, "bulk lock-rejection audit write failed");
            }
            Err(RanchError::ConnectionLocked {
                connection_id,
                connection_name,
                unlock_hint,
            })
        }
        Ok(Err(e)) => Err(e),
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Recent activity — the jobs that ran last, on a queue or a whole connection
// ─────────────────────────────────────────────────────────────────────────────

/// Args for `list_recent_jobs`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListRecentJobsArgs {
    pub connection_id: String,
    /// One queue; `None` = every queue of the connection, merged.
    #[serde(default)]
    pub queue_ref: Option<QueueRef>,
    /// Jobs wanted, clamped to 1–50 (0 = 15).
    #[serde(default)]
    pub limit: u32,
}

/// What ran last: the running jobs, then the latest completed / failed ones.
/// Bounded whatever the size of the queues (see `bullmq_schema::recent`).
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_recent_jobs(
    args: ListRecentJobsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<RecentJobs, RanchErrorDto> {
    list_recent_jobs_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn list_recent_jobs_inner(
    args: ListRecentJobsArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<RecentJobs, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.list_recent_jobs(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Failed jobs across the queues of a connection
// ─────────────────────────────────────────────────────────────────────────────

/// Args for `list_failed_jobs`. All filters are optional and combine.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListFailedJobsArgs {
    pub connection_id: String,
    pub offset: u32,
    /// Page size, clamped to 1–250.
    pub limit: u32,
    /// Exact queue name.
    #[serde(default)]
    pub queue: Option<String>,
    /// Exact job name.
    #[serde(default)]
    pub name: Option<String>,
    /// Case-insensitive substring of the failure reason.
    #[serde(default)]
    pub reason: Option<String>,
    /// Exact cause signature (`FailureCause::signature`).
    #[serde(default)]
    pub signature: Option<String>,
}

/// Everything that failed on a connection, newest first — one view instead of
/// one queue at a time.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_failed_jobs(
    args: ListFailedJobsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<FailedJobsPage, RanchErrorDto> {
    list_failed_jobs_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

/// The failed jobs of a queue, or of the whole connection,
/// grouped by cause.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListFailureCausesArgs {
    pub connection_id: String,
    /// Exact queue name; none = every queue of the connection.
    #[serde(default)]
    pub queue: Option<String>,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_failure_causes(
    args: ListFailureCausesArgs,
    state: tauri::State<'_, AppState>,
) -> Result<FailureCauses, RanchErrorDto> {
    list_failure_causes_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn list_failure_causes_inner(
    args: ListFailureCausesArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<FailureCauses, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.list_failure_causes(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

pub async fn list_failed_jobs_inner(
    args: ListFailedJobsArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<FailedJobsPage, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.list_failed_jobs(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}
