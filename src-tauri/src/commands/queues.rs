//! Queue commands: discovery, schedulers, workers, flows, maintenance,
//! pause and resume, metrics, events and the server behind a connection.
//!
//! Each operation opens the connection's queue backend
//! (`application::ports::backend`) and calls the capability it needs; a
//! backend without it answers `Unsupported`. The Tauri commands below are
//! thin wrappers over the `*_inner` functions, which the HTTP server and
//! the MCP tools call too.

use std::sync::Arc;

pub use crate::contract::queues::*;

#[cfg(feature = "desktop")]
use crate::adapters::progress::TauriProgress;
use crate::app_state::AppState;
use crate::application::ports::backend::EventSink;
use crate::application::ports::{require, Backends, ProgressSink};
use crate::contract::errors::RanchErrorDto;
use crate::contract::jobs::FlowRoots;
use crate::domain::audit::Caller;
use crate::domain::capabilities::Capability;
use crate::domain::errors::RanchError;
use crate::domain::metrics::QueueMetricsQuery;

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_queues(
    args: ListQueuesArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Queue>, RanchErrorDto> {
    list_queues_inner(&args.connection_id, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn list_queues_inner(
    connection_id: &str,
    backends: &dyn Backends,
) -> Result<Vec<Queue>, RanchError> {
    let queues = backends
        .open(connection_id)
        .await?
        .queues()
        .list_queues()
        .await?;
    Ok(queues.into_iter().map(Into::into).collect())
}

// ─────────────────────────────────────────────────────────────────────────────
// list_cron_jobs
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_cron_jobs(
    args: ListCronJobsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<CronJob>, RanchErrorDto> {
    list_cron_jobs_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn list_cron_jobs_inner(
    args: ListCronJobsArgs,
    backends: &dyn Backends,
) -> Result<Vec<CronJob>, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::SchedulesList,
        backend.schedules(),
    )?
    .list_schedulers(args.queue_id.into())
    .await
    .map(|v| v.into_iter().map(Into::into).collect())
}

// ─────────────────────────────────────────────────────────────────────────────
// list_workers
// ─────────────────────────────────────────────────────────────────────────────

/// The BullMQ workers attached to a queue, read from
/// `CLIENT LIST`. A Redis that refuses the command answers
/// `supported: false`, not an error.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_workers(
    args: ListWorkersArgs,
    state: tauri::State<'_, AppState>,
) -> Result<QueueWorkers, RanchErrorDto> {
    list_workers_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn list_workers_inner(
    args: ListWorkersArgs,
    backends: &dyn Backends,
) -> Result<QueueWorkers, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::WorkersList,
        backend.workers(),
    )?
    .list_workers(args.queue_id.into())
    .await
    .map(Into::into)
}

// ─────────────────────────────────────────────────────────────────────────────
// count_queue_workers
// ─────────────────────────────────────────────────────────────────────────────

/// How many workers each queue has, from one `CLIENT LIST`
/// (the sidebar puts queues with jobs waiting and no worker first). UI-only.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn count_queue_workers(
    args: CountQueueWorkersArgs,
    state: tauri::State<'_, AppState>,
) -> Result<QueueWorkerCounts, RanchErrorDto> {
    count_queue_workers_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn count_queue_workers_inner(
    args: CountQueueWorkersArgs,
    backends: &dyn Backends,
) -> Result<QueueWorkerCounts, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::WorkersCount,
        backend.workers(),
    )?
    .count_workers(args.queue_ids.into_iter().map(Into::into).collect())
    .await
    .map(Into::into)
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
    list_flow_roots_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn list_flow_roots_inner(
    args: QueueActionArgs,
    backends: &dyn Backends,
) -> Result<FlowRoots, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(backend.capabilities(), Capability::Flows, backend.flows())?
        .list_flow_roots(args.queue_id.into())
        .await
        .map(Into::into)
}

// ─────────────────────────────────────────────────────────────────────────────
// Job schedulers: preview, create / edit, delete
// ─────────────────────────────────────────────────────────────────────────────

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
    crate::domain::schedule::Schedule::from(schedule.clone())
        .next_runs(now, crate::domain::schedule::PREVIEW_RUNS)
        .map(|next_runs_ms| ScheduleRuns { next_runs_ms })
        .map_err(|reason| RanchError::InvalidInput { reason })
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
    crate::domain::schedule::Schedule::from(args.schedule.clone())
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
    upsert_scheduler_inner(args, state.backends.as_ref(), Caller::User)
        .await
        .map_err(Into::into)
}

pub async fn upsert_scheduler_inner(
    args: UpsertSchedulerArgs,
    backends: &dyn Backends,
    caller: Caller,
) -> Result<SchedulerSaved, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::SchedulesWrite,
        backend.schedules(),
    )?
    .upsert_scheduler(args.queue_id.into(), args.scheduler.into(), caller)
    .await
    .map(Into::into)
}

/// Delete a job scheduler and its pending job.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn remove_scheduler(
    args: SchedulerActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<SchedulerActionResult, RanchErrorDto> {
    remove_scheduler_inner(args, state.backends.as_ref(), Caller::User)
        .await
        .map_err(Into::into)
}

pub async fn remove_scheduler_inner(
    args: SchedulerActionArgs,
    backends: &dyn Backends,
    caller: Caller,
) -> Result<SchedulerActionResult, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::SchedulesWrite,
        backend.schedules(),
    )?
    .remove_scheduler(args.into(), caller)
    .await
    .map(Into::into)
}

// ─────────────────────────────────────────────────────────────────────────────
// Queue maintenance: drain, clean, retry all failed
// ─────────────────────────────────────────────────────────────────────────────

/// How many jobs an operation would touch. Read-only.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn preview_queue_maintenance(
    args: QueueMaintenanceArgs,
    state: tauri::State<'_, AppState>,
) -> Result<MaintenancePreview, RanchErrorDto> {
    preview_queue_maintenance_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn preview_queue_maintenance_inner(
    args: QueueMaintenanceArgs,
    backends: &dyn Backends,
) -> Result<MaintenancePreview, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    let operation = args.operation.into();
    let need = Capability::of_maintenance(&operation);
    require(backend.capabilities(), need, backend.maintenance())?
        .preview(args.queue_id.into(), operation)
        .await
        .map(Into::into)
}

/// Runs a maintenance operation as the UI (or, with `caller`, an agent).
pub async fn run_queue_maintenance_inner(
    args: QueueMaintenanceArgs,
    backends: &dyn Backends,
    progress: Arc<dyn ProgressSink>,
    caller: Caller,
) -> Result<MaintenanceResult, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    let operation = args.operation.into();
    let need = Capability::of_maintenance(&operation);
    require(backend.capabilities(), need, backend.maintenance())?
        .run(args.queue_id.into(), operation, progress, caller)
        .await
        .map(Into::into)
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
            queue_id: args.queue_id,
            operation: QueueMaintenance::Drain {
                include_delayed: args.include_delayed,
            },
        },
        state.backends.as_ref(),
        TauriProgress::sink(app),
        Caller::User,
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
            queue_id: args.queue_id,
            operation: QueueMaintenance::Clean {
                state: args.state,
                older_than_ms: args.older_than_ms,
            },
        },
        state.backends.as_ref(),
        TauriProgress::sink(app),
        Caller::User,
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
            queue_id: args.queue_id,
            operation: QueueMaintenance::RetryAllFailed,
        },
        state.backends.as_ref(),
        TauriProgress::sink(app),
        Caller::User,
    )
    .await
    .map_err(Into::into)
}

// ─────────────────────────────────────────────────────────────────────────────
// Pause / resume queue + scheduler
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn pause_queue(
    args: QueueActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<QueueActionResult, RanchErrorDto> {
    pause_queue_inner(args, state.backends.as_ref(), Caller::User)
        .await
        .map_err(Into::into)
}

pub async fn pause_queue_inner(
    args: QueueActionArgs,
    backends: &dyn Backends,
    caller: Caller,
) -> Result<QueueActionResult, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::QueueControl,
        backend.queue_control(),
    )?
    .pause_queue(args.queue_id.into(), caller)
    .await
    .map(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn resume_queue(
    args: QueueActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<QueueActionResult, RanchErrorDto> {
    resume_queue_inner(args, state.backends.as_ref(), Caller::User)
        .await
        .map_err(Into::into)
}

pub async fn resume_queue_inner(
    args: QueueActionArgs,
    backends: &dyn Backends,
    caller: Caller,
) -> Result<QueueActionResult, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::QueueControl,
        backend.queue_control(),
    )?
    .resume_queue(args.queue_id.into(), caller)
    .await
    .map(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn pause_scheduler(
    args: SchedulerActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<SchedulerActionResult, RanchErrorDto> {
    pause_scheduler_inner(args, state.backends.as_ref(), Caller::User)
        .await
        .map_err(Into::into)
}

pub async fn pause_scheduler_inner(
    args: SchedulerActionArgs,
    backends: &dyn Backends,
    caller: Caller,
) -> Result<SchedulerActionResult, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::SchedulesPause,
        backend.schedules(),
    )?
    .pause_scheduler(args.into(), caller)
    .await
    .map(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn resume_scheduler(
    args: SchedulerActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<SchedulerActionResult, RanchErrorDto> {
    resume_scheduler_inner(args, state.backends.as_ref(), Caller::User)
        .await
        .map_err(Into::into)
}

pub async fn resume_scheduler_inner(
    args: SchedulerActionArgs,
    backends: &dyn Backends,
    caller: Caller,
) -> Result<SchedulerActionResult, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::SchedulesPause,
        backend.schedules(),
    )?
    .resume_scheduler(args.into(), caller)
    .await
    .map(Into::into)
}

// ─────────────────────────────────────────────────────────────────────────────
// 24h observability metrics for the Overview dashboards.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_queue_metrics(
    args: GetQueueMetricsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<QueueMetrics, RanchErrorDto> {
    get_queue_metrics_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn get_queue_metrics_inner(
    args: GetQueueMetricsArgs,
    backends: &dyn Backends,
) -> Result<QueueMetrics, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::MetricsQueue,
        backend.metrics(),
    )?
    .queue_metrics(args.into())
    .await
    .map(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_throughput(
    args: GetThroughputArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Throughput, RanchErrorDto> {
    get_throughput_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn summarize_window(
    args: SummarizeWindowArgs,
    state: tauri::State<'_, AppState>,
) -> Result<WindowSummary, RanchErrorDto> {
    summarize_window_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn summarize_window_inner(
    args: SummarizeWindowArgs,
    backends: &dyn Backends,
) -> Result<WindowSummary, RanchError> {
    if args.to_ms < args.from_ms || args.from_ms < args.range_from_ms {
        return Err(RanchError::InvalidInput {
            reason: "The window must sit inside the visible range.".into(),
        });
    }
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::MetricsTimeline,
        backend.metrics(),
    )?
    .summarize_window(args.into())
    .await
    .map(Into::into)
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
    follow_connection_events_inner(args, state.backends.as_ref(), TauriProgress::sink(app))
        .await
        .map_err(Into::into)
}

pub async fn follow_connection_events_inner(
    args: FollowConnectionEventsArgs,
    backends: &dyn Backends,
    progress: Arc<dyn ProgressSink>,
) -> Result<bool, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    let sink = progress
        .shown()
        .then(|| -> EventSink { Arc::new(move |notice| progress.queue_events(notice)) });
    // A backend that cannot follow answers as the server runtime does:
    // not followed, and the front end keeps polling.
    let events = match backend.events() {
        Some(events) if backend.capabilities().events.follow => events,
        _ => return Ok(false),
    };
    events.follow(args.follow, sink).await
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_topology(
    args: GetTopologyArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Topology, RanchErrorDto> {
    get_topology_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn get_topology_inner(
    args: GetTopologyArgs,
    backends: &dyn Backends,
) -> Result<Topology, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::ServerTopology,
        backend.server(),
    )?
    .topology()
    .await
    .map(Into::into)
}

pub async fn get_throughput_inner(
    args: GetThroughputArgs,
    backends: &dyn Backends,
) -> Result<Throughput, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::MetricsThroughput,
        backend.metrics(),
    )?
    .throughput(args.range.into())
    .await
    .map(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn read_queue_events(
    args: ReadQueueEventsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<QueueEvents, RanchErrorDto> {
    read_queue_events_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn read_queue_events_inner(
    args: ReadQueueEventsArgs,
    backends: &dyn Backends,
) -> Result<QueueEvents, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::EventsRead,
        backend.events(),
    )?
    .read_events(args.into())
    .await
    .map(Into::into)
}

/// The Redis server behind a connection, for the overview's
/// info strip: version, latency, memory, ops/s, clients.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_server_info(
    args: GetServerInfoArgs,
    state: tauri::State<'_, AppState>,
) -> Result<ServerInfo, RanchErrorDto> {
    get_server_info_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn get_server_info_inner(
    args: GetServerInfoArgs,
    backends: &dyn Backends,
) -> Result<ServerInfo, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::ServerInfo,
        backend.server(),
    )?
    .server_info()
    .await
    .map(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_connection_metrics(
    args: GetConnectionMetricsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<QueueMetrics, RanchErrorDto> {
    get_connection_metrics_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn get_connection_metrics_inner(
    args: GetConnectionMetricsArgs,
    backends: &dyn Backends,
) -> Result<QueueMetrics, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    let metrics = require(
        backend.capabilities(),
        Capability::MetricsQueue,
        backend.metrics(),
    )?;

    // Step 1: list queues on the connection.
    let queues = backend.queues().list_queues().await?;

    // Step 2: one queue_metrics per queue, all bucketed against the same
    // `now`, then merged (sums for counts, sample-weighted averages, top-N
    // re-ranked across queues). Sent together: the BullMQ backend runs
    // reads concurrently, so Q queues cost one queue's time.
    let range = args.range;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let replies = crate::core::join::join_all(
        queues
            .into_iter()
            .map(|q| {
                metrics.queue_metrics(QueueMetricsQuery {
                    queue_id: q.id.clone(),
                    range: range.into(),
                    now_ms: Some(now_ms as f64),
                })
            })
            .collect(),
    )
    .await;
    let mut per_queue = Vec::with_capacity(replies.len());
    for reply in replies {
        per_queue.push(reply?);
    }
    Ok(crate::domain::metrics::merge(range.into(), now_ms, &per_queue).into())
}
