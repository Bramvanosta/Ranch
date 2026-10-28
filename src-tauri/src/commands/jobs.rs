//! Job commands: lists, one job and its logs, flows, the job writes and
//! the failed jobs of a connection.
//!
//! Each operation opens the connection's queue backend
//! (`application::ports::backend`) and calls the capability it needs; a
//! backend without it answers `Unsupported`. The Tauri commands below are
//! thin wrappers over the `*_inner` functions, which the HTTP server and
//! the MCP tools call too.

pub use crate::contract::jobs::*;

use std::sync::Arc;

#[cfg(feature = "desktop")]
use crate::adapters::progress::TauriProgress;
use crate::app_state::AppState;
use crate::application::ports::{require, Backends, ProgressSink};
use crate::contract::errors::RanchErrorDto;
use crate::domain::audit::Caller;
use crate::domain::capabilities::Capability;
use crate::domain::errors::RanchError;

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_jobs(
    args: ListJobsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<ListJobsOutput, RanchErrorDto> {
    list_jobs_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn list_jobs_inner(
    args: ListJobsArgs,
    backends: &dyn Backends,
) -> Result<ListJobsOutput, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    let state = backend.capabilities().job_state(args.state.as_str())?;
    Ok(backend.jobs().list_jobs(args.query(state)).await?.into())
}

// ─────────────────────────────────────────────────────────────────────────────
// get_job + get_job_logs
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_job(
    args: GetJobArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Job, RanchErrorDto> {
    get_job_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn get_job_inner(args: GetJobArgs, backends: &dyn Backends) -> Result<Job, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    Ok(backend.jobs().get_job(args.into()).await?.into())
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn get_job_logs(
    args: GetJobLogsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<JobLogsOutput, RanchErrorDto> {
    get_job_logs_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn get_job_logs_inner(
    args: GetJobLogsArgs,
    backends: &dyn Backends,
) -> Result<JobLogsOutput, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    Ok(backend.jobs().get_job_logs(args.into()).await?.into())
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
    get_flow_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn get_flow_inner(
    args: GetJobArgs,
    backends: &dyn Backends,
) -> Result<FlowTree, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(backend.capabilities(), Capability::Flows, backend.flows())?
        .get_flow(args.into())
        .await
        .map(Into::into)
}

// ─────────────────────────────────────────────────────────────────────────────
// add_job
// ─────────────────────────────────────────────────────────────────────────────

/// Enqueue a job. Lock-aware, audited; the write itself
/// is one atomic Lua script (`bullmq_schema::add_job`).
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn add_job(
    args: AddJobArgs,
    state: tauri::State<'_, AppState>,
) -> Result<AddJobResult, RanchErrorDto> {
    add_job_inner(args, state.backends.as_ref(), Caller::User)
        .await
        .map_err(Into::into)
}

pub async fn add_job_inner(
    args: AddJobArgs,
    backends: &dyn Backends,
    caller: Caller,
) -> Result<AddJobResult, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::JobActionsAdd,
        backend.job_actions(),
    )?
    .add_job(args.queue_id.into(), args.job.into(), caller)
    .await
    .map(Into::into)
}

// ─────────────────────────────────────────────────────────────────────────────
// single-job destructive actions (retry / remove / promote)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn retry_job(
    args: JobActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<JobActionResult, RanchErrorDto> {
    retry_job_inner(args, state.backends.as_ref(), Caller::User)
        .await
        .map_err(Into::into)
}

pub async fn retry_job_inner(
    args: JobActionArgs,
    backends: &dyn Backends,
    caller: Caller,
) -> Result<JobActionResult, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::JobActionsSingle,
        backend.job_actions(),
    )?
    .retry_job(args.into(), caller)
    .await
    .map(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn remove_job(
    args: JobActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<JobActionResult, RanchErrorDto> {
    remove_job_inner(args, state.backends.as_ref(), Caller::User)
        .await
        .map_err(Into::into)
}

pub async fn remove_job_inner(
    args: JobActionArgs,
    backends: &dyn Backends,
    caller: Caller,
) -> Result<JobActionResult, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::JobActionsSingle,
        backend.job_actions(),
    )?
    .remove_job(args.into(), caller)
    .await
    .map(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn promote_job(
    args: JobActionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<JobActionResult, RanchErrorDto> {
    promote_job_inner(args, state.backends.as_ref(), Caller::User)
        .await
        .map_err(Into::into)
}

pub async fn promote_job_inner(
    args: JobActionArgs,
    backends: &dyn Backends,
    caller: Caller,
) -> Result<JobActionResult, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::JobActionsSingle,
        backend.job_actions(),
    )?
    .promote_job(args.into(), caller)
    .await
    .map(Into::into)
}

// ─────────────────────────────────────────────────────────────────────────────
// Bulk single-queue destructive actions
// ─────────────────────────────────────────────────────────────────────────────

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
        state.backends.as_ref(),
        TauriProgress::sink(app),
        Caller::User,
    )
    .await
    .map_err(Into::into)
}

pub async fn bulk_retry_jobs_inner(
    args: BulkActionArgs,
    backends: &dyn Backends,
    progress: Arc<dyn ProgressSink>,
    caller: Caller,
) -> Result<BulkActionResult, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::JobActionsBulk,
        backend.job_actions(),
    )?
    .bulk_retry_jobs(args.into(), progress, caller)
    .await
    .map(Into::into)
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
        state.backends.as_ref(),
        TauriProgress::sink(app),
        Caller::User,
    )
    .await
    .map_err(Into::into)
}

pub async fn bulk_remove_jobs_inner(
    args: BulkActionArgs,
    backends: &dyn Backends,
    progress: Arc<dyn ProgressSink>,
    caller: Caller,
) -> Result<BulkActionResult, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::JobActionsBulk,
        backend.job_actions(),
    )?
    .bulk_remove_jobs(args.into(), progress, caller)
    .await
    .map(Into::into)
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
        state.backends.as_ref(),
        TauriProgress::sink(app),
        Caller::User,
    )
    .await
    .map_err(Into::into)
}

pub async fn bulk_promote_jobs_inner(
    args: BulkActionArgs,
    backends: &dyn Backends,
    progress: Arc<dyn ProgressSink>,
    caller: Caller,
) -> Result<BulkActionResult, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    require(
        backend.capabilities(),
        Capability::JobActionsBulk,
        backend.job_actions(),
    )?
    .bulk_promote_jobs(args.into(), progress, caller)
    .await
    .map(Into::into)
}

// ─────────────────────────────────────────────────────────────────────────────
// Recent activity: the jobs that ran last, on a queue or a whole connection
// ─────────────────────────────────────────────────────────────────────────────

/// What ran last: the running jobs, then the latest completed / failed ones.
/// Bounded whatever the size of the queues (see `bullmq_schema::recent`).
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_recent_jobs(
    args: ListRecentJobsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<RecentJobs, RanchErrorDto> {
    list_recent_jobs_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn list_recent_jobs_inner(
    args: ListRecentJobsArgs,
    backends: &dyn Backends,
) -> Result<RecentJobs, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    Ok(backend.jobs().list_recent_jobs(args.into()).await?.into())
}

// ─────────────────────────────────────────────────────────────────────────────
// Failed jobs across the queues of a connection
// ─────────────────────────────────────────────────────────────────────────────

/// Everything that failed on a connection, newest first: one view instead of
/// one queue at a time.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_failed_jobs(
    args: ListFailedJobsArgs,
    state: tauri::State<'_, AppState>,
) -> Result<FailedJobsPage, RanchErrorDto> {
    list_failed_jobs_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_failure_causes(
    args: ListFailureCausesArgs,
    state: tauri::State<'_, AppState>,
) -> Result<FailureCauses, RanchErrorDto> {
    list_failure_causes_inner(args, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn list_failure_causes_inner(
    args: ListFailureCausesArgs,
    backends: &dyn Backends,
) -> Result<FailureCauses, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    Ok(backend.jobs().list_failure_causes(args.queue).await?.into())
}

pub async fn list_failed_jobs_inner(
    args: ListFailedJobsArgs,
    backends: &dyn Backends,
) -> Result<FailedJobsPage, RanchError> {
    let backend = backends.open(&args.connection_id).await?;
    Ok(backend.jobs().list_failed_jobs(args.into()).await?.into())
}
