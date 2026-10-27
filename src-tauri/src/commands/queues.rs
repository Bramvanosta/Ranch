//! Queue commands.
//!
//! All queue/job IO routes through `core::connection_actor`. This file
//! is the thin command-layer wrapper: resolves the connection's config
//! (SQLite + keychain), dispatches to the registry, plumbs the result
//! back through specta's `Result<_, RanchErrorDto>` boundary.
//!
//! ## Discovery
//!
//! - `list_queues({ connectionId }) -> Vec<Queue>` — discovers BullMQ
//!   queues via `SCAN MATCH *:meta`, fetches counters via MULTI/EXEC.
//!
//! ## Schedulers
//!
//! - `list_cron_jobs({ connectionId, queueRef }) -> Vec<CronJob>` —
//!   reads the `bull:{queue}:repeat` ZSET + pipelines HGETALL per
//!   scheduler key. Surfaces every scheduler's pattern (or every-ms),
//!   next-run timestamp, tz, and iteration count.

use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::SqlitePool;
use std::sync::Arc;

use crate::app_state::AppState;
use crate::commands::connections::resolve_connection_config;
use crate::core::bullmq_schema::{
    CleanState, CronJob, FlowRoots, MaintenancePreview, MaintenanceResult, MetricsRange, Queue,
    QueueMaintenance, QueueMetrics, QueueRef, QueueWorkerCounts, QueueWorkers, Schedule,
    SchedulerSaved, SchedulerSpec, Throughput,
};
use crate::core::connection_actor::ConnectionRegistry;
use crate::core::errors::{RanchError, RanchErrorDto};
use crate::core::keychain::KeychainAdapter;

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListQueuesArgs {
    pub connection_id: String,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_queues(
    args: ListQueuesArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Queue>, RanchErrorDto> {
    list_queues_inner(
        &args.connection_id,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn list_queues_inner(
    connection_id: &str,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<Vec<Queue>, RanchError> {
    // 1. Resolve the connection's config (SQL + keychain) via shared helper.
    let config = resolve_connection_config(connection_id, pool, keychain).await?;

    // 2. Dispatch via the registry (lazy-spawns actor on first call).
    //    Flatten the double-Result: actor-level failures (mailbox closed)
    //    surface as InvalidInput; schema/redis errors carry through.
    match registry.list_queues(connection_id, &config).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// list_cron_jobs
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListCronJobsArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_cron_jobs(
    args: ListCronJobsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<CronJob>, RanchErrorDto> {
    list_cron_jobs_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn list_cron_jobs_inner(
    args: ListCronJobsArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<Vec<CronJob>, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.list_cron_jobs(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// list_workers
// ─────────────────────────────────────────────────────────────────────────────

/// Args for `list_workers`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListWorkersArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
}

/// The BullMQ workers attached to a queue, read from
/// `CLIENT LIST`. A Redis that refuses the command answers
/// `supported: false` — not an error.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_workers(
    args: ListWorkersArgs,
    state: tauri::State<'_, AppState>,
) -> Result<QueueWorkers, RanchErrorDto> {
    list_workers_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn list_workers_inner(
    args: ListWorkersArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<QueueWorkers, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.list_workers(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// count_queue_workers
// ─────────────────────────────────────────────────────────────────────────────

/// Args for `count_queue_workers`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CountQueueWorkersArgs {
    pub connection_id: String,
    pub queue_refs: Vec<QueueRef>,
}

/// How many workers each queue has, from one `CLIENT LIST`
/// (the sidebar puts queues with jobs waiting and no worker first). UI-only.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn count_queue_workers(
    args: CountQueueWorkersArgs,
    state: tauri::State<'_, AppState>,
) -> Result<QueueWorkerCounts, RanchErrorDto> {
    count_queue_workers_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn count_queue_workers_inner(
    args: CountQueueWorkersArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<QueueWorkerCounts, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.count_queue_workers(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Flows of a queue
// ─────────────────────────────────────────────────────────────────────────────

/// Flow roots found in a queue (bounded discovery).
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_flow_roots(
    args: QueueActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<FlowRoots, RanchErrorDto> {
    list_flow_roots_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn list_flow_roots_inner(
    args: QueueActionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<FlowRoots, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.list_flow_roots(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Job schedulers: preview, create / edit, delete
// ─────────────────────────────────────────────────────────────────────────────

/// Args for `upsert_scheduler`. Saving an existing `scheduler.id` edits it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpsertSchedulerArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
    pub scheduler: SchedulerSpec,
}

/// The next runs of a schedule — what the sheet shows while typing.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRuns {
    /// Milliseconds since the epoch, soonest first.
    pub next_runs_ms: Vec<i64>,
}

/// Validate a schedule and compute its next 5 runs. Pure: no
/// Redis, no connection. An invalid pattern / timezone / interval comes
/// back as an `invalidInput` whose message is shown under the field.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn preview_schedule(schedule: Schedule) -> Result<ScheduleRuns, RanchErrorDto> {
    preview_schedule_inner(&schedule, chrono::Utc::now()).map_err(Into::into)
}

pub fn preview_schedule_inner(
    schedule: &Schedule,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<ScheduleRuns, RanchError> {
    schedule
        .next_runs(now, crate::core::bullmq_schema::scheduler::PREVIEW_RUNS)
        .map(|next_runs_ms| ScheduleRuns { next_runs_ms })
        .map_err(|reason| RanchError::InvalidInput { reason })
}

/// The runs of a schedule in a window (the Schedulers tab's
/// "Next 24 hours"), at most `max` (≤ 500).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRunsWithinArgs {
    pub schedule: Schedule,
    /// Milliseconds after now: 86 400 000 for the next 24 hours.
    pub horizon_ms: f64,
    /// For `every`: the scheduler's next run, so the runs keep its grid.
    #[serde(default)]
    pub first_ms: Option<f64>,
    #[serde(default)]
    pub max: Option<u32>,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn schedule_runs_within(
    args: ScheduleRunsWithinArgs,
) -> Result<ScheduleRuns, RanchErrorDto> {
    schedule_runs_within_inner(&args, chrono::Utc::now()).map_err(Into::into)
}

pub fn schedule_runs_within_inner(
    args: &ScheduleRunsWithinArgs,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<ScheduleRuns, RanchError> {
    let horizon = chrono::Duration::milliseconds(args.horizon_ms.max(0.0) as i64);
    args.schedule
        .runs_within(
            now,
            now + horizon,
            args.first_ms.map(|v| v as i64),
            args.max.unwrap_or(500) as usize,
        )
        .map(|next_runs_ms| ScheduleRuns { next_runs_ms })
        .map_err(|reason| RanchError::InvalidInput { reason })
}

/// Create or replace a job scheduler.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn upsert_scheduler(
    args: UpsertSchedulerArgs,
    state: tauri::State<'_, AppState>,
) -> Result<SchedulerSaved, RanchErrorDto> {
    upsert_scheduler_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
        crate::core::connection_actor::AuditAttribution::ui(),
    )
    .await
    .map_err(Into::into)
}

pub async fn upsert_scheduler_inner(
    args: UpsertSchedulerArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
    attribution: crate::core::connection_actor::AuditAttribution,
) -> Result<SchedulerSaved, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry
        .upsert_scheduler(&id, &config, args, attribution)
        .await
    {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

/// Delete a job scheduler and its pending job.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn remove_scheduler(
    args: SchedulerActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<SchedulerActionResult, RanchErrorDto> {
    remove_scheduler_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
        crate::core::connection_actor::AuditAttribution::ui(),
    )
    .await
    .map_err(Into::into)
}

pub async fn remove_scheduler_inner(
    args: SchedulerActionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
    attribution: crate::core::connection_actor::AuditAttribution,
) -> Result<SchedulerActionResult, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry
        .remove_scheduler(&id, &config, args, attribution)
        .await
    {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Queue maintenance: drain, clean, retry all failed
// ─────────────────────────────────────────────────────────────────────────────

/// A maintenance operation on one queue — for the preview and for the run.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueMaintenanceArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
    pub operation: QueueMaintenance,
}

/// Args for `drain_queue`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DrainQueueArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
    /// Also remove delayed jobs (BullStudio / BullMQ default: yes).
    pub include_delayed: bool,
}

/// Args for `clean_queue`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanQueueArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
    pub state: CleanState,
    /// Only jobs that finished more than this long ago. `0` = all.
    pub older_than_ms: u64,
}

/// How many jobs an operation would touch. Read-only.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn preview_queue_maintenance(
    args: QueueMaintenanceArgs,
    state: tauri::State<'_, AppState>,
) -> Result<MaintenancePreview, RanchErrorDto> {
    preview_queue_maintenance_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn preview_queue_maintenance_inner(
    args: QueueMaintenanceArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<MaintenancePreview, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.preview_queue_maintenance(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

/// Runs a maintenance operation as the UI (or, with `attribution`, an agent).
pub async fn run_queue_maintenance_inner(
    args: QueueMaintenanceArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
    app: Option<&crate::core::events::AppHandle>,
    attribution: crate::core::connection_actor::AuditAttribution,
) -> Result<MaintenanceResult, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry
        .run_queue_maintenance(&id, &config, args, app.cloned(), attribution)
        .await
    {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

/// Remove every job that has not started.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn drain_queue(
    args: DrainQueueArgs,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<MaintenanceResult, RanchErrorDto> {
    run_queue_maintenance_inner(
        QueueMaintenanceArgs {
            connection_id: args.connection_id,
            queue_ref: args.queue_ref,
            operation: QueueMaintenance::Drain {
                include_delayed: args.include_delayed,
            },
        },
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
        Some(&app),
        crate::core::connection_actor::AuditAttribution::ui(),
    )
    .await
    .map_err(Into::into)
}

/// Remove finished jobs older than a cutoff.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn clean_queue(
    args: CleanQueueArgs,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<MaintenanceResult, RanchErrorDto> {
    run_queue_maintenance_inner(
        QueueMaintenanceArgs {
            connection_id: args.connection_id,
            queue_ref: args.queue_ref,
            operation: QueueMaintenance::Clean {
                state: args.state,
                older_than_ms: args.older_than_ms,
            },
        },
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
        Some(&app),
        crate::core::connection_actor::AuditAttribution::ui(),
    )
    .await
    .map_err(Into::into)
}

/// Put every failed job of the queue back in line.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn retry_all_failed(
    args: QueueActionArgs,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<MaintenanceResult, RanchErrorDto> {
    run_queue_maintenance_inner(
        QueueMaintenanceArgs {
            connection_id: args.connection_id,
            queue_ref: args.queue_ref,
            operation: QueueMaintenance::RetryAllFailed,
        },
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
        Some(&app),
        crate::core::connection_actor::AuditAttribution::ui(),
    )
    .await
    .map_err(Into::into)
}

// ─────────────────────────────────────────────────────────────────────────────
// Pause / resume queue + scheduler
// ─────────────────────────────────────────────────────────────────────────────

/// Args for `pause_queue` / `resume_queue`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueActionArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
}

/// Result of `pause_queue` / `resume_queue`. `is_paused` is the
/// post-call state. `already_in_state` is true when the call was a
/// successful no-op (the queue was already in the target state) —
/// matches the idempotency contract: pause-when-paused returns
/// `Ok({ is_paused: true, already_in_state: true })`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueActionResult {
    pub is_paused: bool,
    pub already_in_state: bool,
}

/// Args for `pause_scheduler` / `resume_scheduler`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerActionArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
    pub scheduler_key: String,
}

/// Result of `pause_scheduler` / `resume_scheduler`. Same shape as
/// `QueueActionResult` but with `paused` rather than `is_paused` so
/// the field name matches the `CronJob.paused` UI binding.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerActionResult {
    pub paused: bool,
    pub already_in_state: bool,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn pause_queue(
    args: QueueActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<QueueActionResult, RanchErrorDto> {
    pause_queue_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn pause_queue_inner(
    args: QueueActionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<QueueActionResult, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.pause_queue(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn resume_queue(
    args: QueueActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<QueueActionResult, RanchErrorDto> {
    resume_queue_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn resume_queue_inner(
    args: QueueActionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<QueueActionResult, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.resume_queue(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn pause_scheduler(
    args: SchedulerActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<SchedulerActionResult, RanchErrorDto> {
    pause_scheduler_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn pause_scheduler_inner(
    args: SchedulerActionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<SchedulerActionResult, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.pause_scheduler(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn resume_scheduler(
    args: SchedulerActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<SchedulerActionResult, RanchErrorDto> {
    resume_scheduler_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn resume_scheduler_inner(
    args: SchedulerActionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<SchedulerActionResult, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.resume_scheduler(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 24h observability metrics for the Overview dashboards.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GetQueueMetricsArgs {
    pub connection_id: String,
    pub queue_ref: QueueRef,
    /// The window (5m … 7d). Defaults to 24h.
    #[serde(default)]
    pub range: MetricsRange,
    /// "now" (ms), so that every queue of a connection-level call
    /// is bucketed against the same instant. `None` = the actor's clock.
    #[serde(default)]
    pub now_ms: Option<f64>,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_queue_metrics(
    args: GetQueueMetricsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<QueueMetrics, RanchErrorDto> {
    get_queue_metrics_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn get_queue_metrics_inner(
    args: GetQueueMetricsArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<QueueMetrics, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.get_queue_metrics(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

/// Completed / failed over time for every queue of a
/// connection, from native metrics. Cheap: two reads per queue, one
/// pipeline, no job sampling.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GetThroughputArgs {
    pub connection_id: String,
    /// 1h, 6h or 24h are what the screens ask for; any window works.
    #[serde(default)]
    pub range: MetricsRange,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_throughput(
    args: GetThroughputArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Throughput, RanchErrorDto> {
    get_throughput_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

/// The Timeline's dragged window: done, failed, wait p95
/// against the rest of the visible range, failures by cause.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SummarizeWindowArgs {
    pub connection_id: String,
    /// The window, ms since epoch.
    pub from_ms: i64,
    pub to_ms: i64,
    /// Start of the visible range: "usual" is the range outside the window.
    pub range_from_ms: i64,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn summarize_window(
    args: SummarizeWindowArgs,
    state: tauri::State<'_, AppState>,
) -> Result<crate::core::bullmq_schema::timeline::WindowSummary, RanchErrorDto> {
    summarize_window_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn summarize_window_inner(
    args: SummarizeWindowArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<crate::core::bullmq_schema::timeline::WindowSummary, RanchError> {
    if args.to_ms < args.from_ms || args.from_ms < args.range_from_ms {
        return Err(RanchError::InvalidInput {
            reason: "The window must sit inside the visible range.".into(),
        });
    }
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.summarize_window(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

/// Follow (or stop following) a connection's BullMQ event
/// streams. Each window that follows counts; the reading stops when the
/// last one stops.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FollowConnectionEventsArgs {
    pub connection_id: String,
    pub follow: bool,
}

/// The `queue:events` event: what BullMQ wrote to the queues'
/// event streams since the last one, per queue.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueEventsNotice {
    pub connection_id: String,
    pub queues: Vec<QueueEventsSeen>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueEventsSeen {
    pub queue_ref: QueueRef,
    /// Event names seen (`added`, `completed`, `failed`, …), each once.
    pub events: Vec<String>,
    /// Entries read, capped by the read's COUNT.
    pub count: u32,
}

/// Whether the connection's events are now followed: `false`
/// in the server runtime (no window to send them to: it keeps polling).
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn follow_connection_events(
    args: FollowConnectionEventsArgs,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<bool, RanchErrorDto> {
    follow_connection_events_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
        Some(app),
    )
    .await
    .map_err(Into::into)
}

pub async fn follow_connection_events_inner(
    args: FollowConnectionEventsArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
    app: Option<crate::core::events::AppHandle>,
) -> Result<bool, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let sink = app.map(|app| -> crate::core::connection_actor::EventSink {
        Arc::new(move |notice| crate::core::events::emit(&app, "queue:events", notice))
    });
    registry
        .follow_events(&args.connection_id, &config, args.follow, sink)
        .await
        .map_err(|e| RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        })
}

/// Flow links between queues, added per minute, oldest waiting.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GetTopologyArgs {
    pub connection_id: String,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_topology(
    args: GetTopologyArgs,
    state: tauri::State<'_, AppState>,
) -> Result<crate::core::bullmq_schema::topology::Topology, RanchErrorDto> {
    get_topology_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn get_topology_inner(
    args: GetTopologyArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<crate::core::bullmq_schema::topology::Topology, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    match registry.get_topology(&args.connection_id, &config).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

pub async fn get_throughput_inner(
    args: GetThroughputArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<Throughput, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.get_throughput(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

/// What happened on the connection's queues since the last
/// read (their `…:events` streams), newest first, capped. The overview asks
/// while it is on screen and stops when it is not.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReadQueueEventsArgs {
    pub connection_id: String,
    /// Where each queue's last read stopped; a queue without one is read
    /// from its last few events.
    #[serde(default)]
    pub cursors: Vec<crate::core::bullmq_schema::events::EventCursor>,
    /// Events wanted at most (default 100).
    #[serde(default)]
    pub limit: Option<u32>,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn read_queue_events(
    args: ReadQueueEventsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<crate::core::bullmq_schema::events::QueueEvents, RanchErrorDto> {
    read_queue_events_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn read_queue_events_inner(
    args: ReadQueueEventsArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<crate::core::bullmq_schema::events::QueueEvents, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.read_queue_events(&id, &config, args).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

/// The Redis server behind a connection, for the overview's
/// info strip: version, latency, memory, ops/s, clients.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_server_info(
    args: GetServerInfoArgs,
    state: tauri::State<'_, AppState>,
) -> Result<crate::core::redis_info::ServerInfo, RanchErrorDto> {
    get_server_info_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GetServerInfoArgs {
    pub connection_id: String,
}

pub async fn get_server_info_inner(
    args: GetServerInfoArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<crate::core::redis_info::ServerInfo, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();
    match registry.get_server_info(&id, &config).await {
        Ok(inner) => inner,
        Err(e) => Err(RanchError::InvalidInput {
            reason: format!("Connection actor error: {e}"),
        }),
    }
}

/// connection-level overview that aggregates metrics
/// across every queue on the connection. Frontend uses this for the
/// env-level dashboard at `/connections/$id/`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GetConnectionMetricsArgs {
    pub connection_id: String,
    /// The window (5m … 7d). Defaults to 24h.
    #[serde(default)]
    pub range: MetricsRange,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_connection_metrics(
    args: GetConnectionMetricsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<QueueMetrics, RanchErrorDto> {
    get_connection_metrics_inner(
        args,
        state.db.pool(),
        &state.keychain,
        state.connections.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn get_connection_metrics_inner(
    args: GetConnectionMetricsArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: &ConnectionRegistry,
) -> Result<QueueMetrics, RanchError> {
    let config = resolve_connection_config(&args.connection_id, pool, keychain).await?;
    let id = args.connection_id.clone();

    // Step 1: list queues on the connection.
    let queues = match registry.list_queues(&id, &config).await {
        Ok(inner) => inner?,
        Err(e) => {
            return Err(RanchError::InvalidInput {
                reason: format!("Connection actor error: {e}"),
            });
        }
    };

    // Step 2: one get_queue_metrics per queue, all bucketed against the same
    // `now`, then merged (sums for counts, sample-weighted averages, top-N
    // re-ranked across queues). Sent together: the actor runs
    // reads concurrently, so Q queues cost one queue's time.
    let range = args.range;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let replies = crate::core::join::join_all(
        queues
            .into_iter()
            .map(|q| {
                let args = GetQueueMetricsArgs {
                    connection_id: id.clone(),
                    queue_ref: q.queue_ref.clone(),
                    range,
                    now_ms: Some(now_ms as f64),
                };
                registry.get_queue_metrics(&id, &config, args)
            })
            .collect(),
    )
    .await;
    let mut per_queue = Vec::with_capacity(replies.len());
    for reply in replies {
        match reply {
            Ok(inner) => per_queue.push(inner?),
            Err(e) => {
                return Err(RanchError::InvalidInput {
                    reason: format!("Connection actor error: {e}"),
                });
            }
        }
    }
    Ok(crate::core::bullmq_schema::metrics::merge(
        range, now_ms, &per_queue,
    ))
}
