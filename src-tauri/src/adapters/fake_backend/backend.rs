//! One fake connection: the port's families over the in-memory world.
//!
//! Every write holds the world for its whole length, so writes and lock
//! changes on a connection happen one at a time, and runs the write
//! pipeline around its change: authorize against the live lock (or the
//! caller's approval), act, record the outcome, then reply. What a write
//! refuses before acting (a job in the wrong state, an unknown scheduler)
//! is not recorded; a bulk action refused by the lock is, as one row.

use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use tokio::sync::Mutex;

use super::model::{job_key, queue_name, FakeJob, FakeQueue, FakeScheduler, World};
use crate::adapters::bullmq_redis::bullmq_schema::add_job::{Placement, PreparedJob};
use crate::adapters::bullmq_redis::bullmq_schema::flow::is_blocked;
use crate::adapters::bullmq_redis::bullmq_schema::recent::{clamp_limit, order_recent};
use crate::adapters::bullmq_redis::bullmq_schema::states;
use crate::adapters::bullmq_redis::bullmq_schema::{
    CleanState, CronJob, FlowNode, FlowRoot, FlowRoots, FlowTree, Job, JobState,
    MaintenancePreview, MaintenanceResult, QueueMaintenance, QueueWorker, QueueWorkerCount,
    QueueWorkerCounts, QueueWorkers, RecentJobs, SchedulerSaved,
};
use crate::application::ports::backend::{
    ConnectionControl, Flows, JobActions, JobReads, Maintenance, QueueBackend, QueueControl,
    QueueReads, Schedules, Workers,
};
use crate::application::ports::ProgressSink;
use crate::application::{ConnectionStamp, WritePipeline};
use crate::domain::audit::{AuditAction, AuditOutcome, AuditTarget, Caller};
use crate::domain::capabilities::{
    Capabilities, JobActionSupport, MaintenanceSupport, ScheduleSupport, WorkerSupport,
};
use crate::domain::errors::RanchError;
use crate::domain::failures::{
    group_by_cause, merge_candidates, page_of, summarise_types, FailedCandidate, FailedJobsPage,
    FailedJobsQuery, FailedQueueCount, FailureCauses,
};
use crate::domain::jobs::{
    AddJobResult, BulkActionResult, BulkFailure, JobActionResult, JobBatch, JobLogs, JobLogsQuery,
    JobPage, JobPageQuery, JobTarget, NewJob, RecentJobsQuery,
};
use crate::domain::lock::ReadOnlyState;
use crate::domain::probe::PingReport;
use crate::domain::queues::{Queue, QueueActionResult, QueueId};
use crate::domain::schedule::{Schedule, SchedulerActionResult, SchedulerSpec, SchedulerTarget};

/// The `memory` kind keeps every state but `stalled`, and does what an
/// in-memory store can: no metrics, events or server to report on.
pub(super) static MEMORY_CAPABILITIES: LazyLock<Capabilities> = LazyLock::new(|| Capabilities {
    job_states: vec![
        JobState::Waiting,
        JobState::Active,
        JobState::Completed,
        JobState::Failed,
        JobState::Delayed,
        states::PRIORITIZED,
        states::WAITING_CHILDREN,
        JobState::Paused,
    ],
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
    ..Default::default()
});

pub(super) struct FakeBackend {
    pub stamp: ConnectionStamp,
    /// Its kind's, from the spec the fake was made with. A family is
    /// handed out only when declared.
    pub capabilities: Capabilities,
    pub writes: Arc<WritePipeline>,
    pub world: Mutex<World>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The queue a pause or resume lands on. Pausing a queue nobody declared
/// makes it, as BullMQ's write makes its `meta` key.
fn queue_entry<'w>(world: &'w mut World, name: &str) -> &'w mut FakeQueue {
    world
        .queues
        .entry(name.to_string())
        .or_insert_with(|| FakeQueue::new(name))
}

fn missing_queue(name: &str) -> RanchError {
    RanchError::InvalidInput {
        reason: format!("Queue \"{name}\" does not exist on this connection"),
    }
}

/// The queue an argument names: its id, as the fake made it, and its name.
fn queue_of(id: &QueueId) -> Result<(QueueId, String), RanchError> {
    let name = queue_name(id.as_str())?;
    Ok((id.clone(), name))
}

impl FakeBackend {
    /// A write refused by the lock leaves a failure row for its target,
    /// as the real actor does. Best effort: failing to record it never
    /// hides the refusal.
    async fn authorize(
        &self,
        world: &World,
        caller: &Caller,
        tool: &str,
        (action, target): (AuditAction, AuditTarget),
    ) -> Result<(), RanchError> {
        let decision = self
            .writes
            .authorize(&self.stamp, world.lock, caller, tool)
            .await;
        if let Err(RanchError::ConnectionLocked { .. }) = &decision {
            let reason = Some("Connection locked".to_string());
            let recorded = self
                .writes
                .record(
                    &self.stamp,
                    caller,
                    action,
                    target,
                    AuditOutcome::Failure,
                    reason,
                )
                .await;
            if let Err(e) = recorded {
                tracing::warn!(error = %e, "lock-rejection audit write failed");
            }
        }
        decision
    }

    async fn record(
        &self,
        caller: &Caller,
        action: AuditAction,
        target: AuditTarget,
        reason: Option<String>,
    ) -> Result<(), RanchError> {
        self.writes
            .record(
                &self.stamp,
                caller,
                action,
                target,
                AuditOutcome::Success,
                reason,
            )
            .await
    }

    fn invalid_state(
        &self,
        action: &str,
        args: &JobTarget,
        current: Option<JobState>,
        required: JobState,
    ) -> RanchError {
        RanchError::InvalidJobState {
            action: action.to_string(),
            connection_id: self.stamp.id.clone(),
            queue_name: queue_name(args.queue_id.as_str()).unwrap_or_default(),
            job_id: args.job_id.clone(),
            current_state: current.unwrap_or(states::UNKNOWN),
            required_state: required,
        }
    }

    /// One job moved from `from` back in line, by a single write.
    async fn move_one(
        &self,
        args: JobTarget,
        caller: Caller,
        (tool, verb, action): (&str, &str, AuditAction),
        from: JobState,
    ) -> Result<JobActionResult, RanchError> {
        let mut world = self.world.lock().await;
        let (queue_id, name) = queue_of(&args.queue_id)?;
        let target = AuditTarget::job(&queue_id, &name, &args.job_id);
        self.authorize(&world, &caller, tool, (action, target.clone()))
            .await?;
        let queue = world.queues.get_mut(&name);
        let paused = queue.as_ref().is_some_and(|q| q.paused);
        match queue.and_then(|q| q.job_mut(&args.job_id)) {
            Some(job) if job.state == from => {
                job.state = FakeJob::ready_state(paused);
                job.failed_reason = None;
                job.finished_on = None;
            }
            other => {
                let current = other.map(|j| j.state);
                return Err(self.invalid_state(verb, &args, current, from));
            }
        }
        self.record(&caller, action, target, None).await?;
        Ok(JobActionResult {
            job_id: args.job_id,
            new_state: Some(JobState::Waiting),
            action: verb.to_string(),
        })
    }

    /// One authorization, one change per job, one audit row.
    async fn bulk(
        &self,
        args: JobBatch,
        caller: Caller,
        (tool, verb, action): (&str, &str, AuditAction),
        act: fn(&mut FakeQueue, &str) -> Result<(), String>,
    ) -> Result<BulkActionResult, RanchError> {
        let mut world = self.world.lock().await;
        let total = args.job_ids.len() as u32;
        let (queue_id, name) = queue_of(&args.queue_id)?;
        let target = AuditTarget::bulk_jobs(&queue_id, &name, total);
        self.authorize(&world, &caller, tool, (action, target.clone()))
            .await?;
        let mut undeclared = FakeQueue::new(&name);
        let queue = world.queues.get_mut(&name).unwrap_or(&mut undeclared);
        let mut failed = Vec::new();
        for id in &args.job_ids {
            if let Err(reason) = act(queue, id) {
                failed.push(BulkFailure {
                    job_id: id.clone(),
                    reason,
                });
            }
        }
        let succeeded = total - failed.len() as u32;
        let (outcome, reason) = if failed.is_empty() {
            (AuditOutcome::Success, None)
        } else {
            let reason = format!("{succeeded} of {total} succeeded");
            (AuditOutcome::Failure, Some(reason))
        };
        self.writes
            .record(&self.stamp, &caller, action, target, outcome, reason)
            .await?;
        Ok(BulkActionResult {
            action: verb.to_string(),
            total,
            succeeded,
            failed,
            cancelled: false,
        })
    }
}

fn retry_one(queue: &mut FakeQueue, id: &str) -> Result<(), String> {
    let paused = queue.paused;
    let job = queue.job_mut(id).ok_or("job not found")?;
    if job.state != JobState::Failed {
        return Err("not in failed state".to_string());
    }
    job.state = FakeJob::ready_state(paused);
    job.failed_reason = None;
    job.finished_on = None;
    Ok(())
}

fn promote_one(queue: &mut FakeQueue, id: &str) -> Result<(), String> {
    let paused = queue.paused;
    let job = queue.job_mut(id).ok_or("job not found")?;
    if job.state != JobState::Delayed {
        return Err("not in delayed state".to_string());
    }
    job.state = FakeJob::ready_state(paused);
    Ok(())
}

fn remove_one(queue: &mut FakeQueue, id: &str) -> Result<(), String> {
    queue
        .remove_job(id)
        .map(|_| ())
        .ok_or("job not found".into())
}

impl QueueBackend for FakeBackend {
    fn stamp(&self) -> &ConnectionStamp {
        &self.stamp
    }
    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
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
        self.capabilities.job_actions.any().then_some(self)
    }
    fn maintenance(&self) -> Option<&dyn Maintenance> {
        self.capabilities.maintenance.any().then_some(self)
    }
    fn queue_control(&self) -> Option<&dyn QueueControl> {
        self.capabilities.queue_control.then_some(self)
    }
    fn schedules(&self) -> Option<&dyn Schedules> {
        self.capabilities.schedules.any().then_some(self)
    }
    fn workers(&self) -> Option<&dyn Workers> {
        self.capabilities.workers.any().then_some(self)
    }
    fn flows(&self) -> Option<&dyn Flows> {
        self.capabilities.flows.then_some(self)
    }
}

#[async_trait]
impl ConnectionControl for FakeBackend {
    async fn ping(&self) -> Result<PingReport, RanchError> {
        Ok(PingReport {
            success: true,
            latency_ms: Some(0),
            error: None,
        })
    }
    async fn lock_state(&self) -> Result<ReadOnlyState, RanchError> {
        Ok(self.world.lock().await.lock)
    }
    async fn lock(&self) -> Result<ReadOnlyState, RanchError> {
        let mut world = self.world.lock().await;
        world.lock = ReadOnlyState::Locked;
        Ok(world.lock)
    }
    async fn unlock(&self) -> Result<ReadOnlyState, RanchError> {
        let mut world = self.world.lock().await;
        world.lock = ReadOnlyState::Unlocked;
        Ok(world.lock)
    }
}

#[async_trait]
impl QueueReads for FakeBackend {
    async fn list_queues(&self) -> Result<Vec<Queue>, RanchError> {
        let world = self.world.lock().await;
        Ok(world.queues.values().map(FakeQueue::to_queue).collect())
    }
}

impl FakeBackend {
    fn failed_candidates(world: &World) -> Vec<FailedCandidate> {
        merge_candidates(
            world
                .queues
                .values()
                .flat_map(FakeQueue::failed_candidates)
                .collect(),
        )
    }
}

#[async_trait]
impl JobReads for FakeBackend {
    async fn list_jobs(&self, args: JobPageQuery) -> Result<JobPage, RanchError> {
        let world = self.world.lock().await;
        let Some(queue) = world.queues.get(&queue_name(args.queue_id.as_str())?) else {
            return Ok(JobPage {
                jobs: Vec::new(),
                total: 0,
                state: args.state,
                has_more: false,
                scanned: None,
                scan_capped: false,
            });
        };
        let search = args.search.as_deref().map(str::to_lowercase);
        let reason = args.reason.as_deref().map(str::to_lowercase);
        let wanted = args.state;
        let in_state: Vec<&FakeJob> = queue.jobs.iter().filter(|j| j.state == wanted).collect();
        let mut matching: Vec<&FakeJob> = in_state
            .iter()
            .copied()
            .filter(|j| {
                search.as_ref().is_none_or(|s| {
                    j.id.to_lowercase().contains(s) || j.name.to_lowercase().contains(s)
                })
            })
            .filter(|j| {
                reason.as_ref().is_none_or(|r| {
                    j.failed_reason
                        .as_deref()
                        .is_some_and(|f| f.to_lowercase().contains(r))
                })
            })
            .collect();
        if args.newest_first {
            matching.reverse();
        }
        let total = matching.len() as u64;
        let jobs: Vec<Job> = matching
            .into_iter()
            .skip(args.offset as usize)
            .take(args.limit as usize)
            .map(|j| j.to_job(queue))
            .collect();
        let searched = search.is_some() || reason.is_some();
        Ok(JobPage {
            has_more: args.offset + (jobs.len() as u64) < total,
            jobs,
            total,
            state: args.state,
            scanned: searched.then_some(in_state.len() as u64),
            scan_capped: false,
        })
    }

    async fn get_job(&self, args: JobTarget) -> Result<Job, RanchError> {
        let world = self.world.lock().await;
        world
            .queues
            .get(&queue_name(args.queue_id.as_str())?)
            .and_then(|q| q.job(&args.job_id).map(|j| j.to_job(q)))
            .ok_or(RanchError::JobNotFound {
                queue: queue_name(args.queue_id.as_str())?,
                id: args.job_id,
            })
    }

    async fn get_job_logs(&self, args: JobLogsQuery) -> Result<JobLogs, RanchError> {
        let world = self.world.lock().await;
        let logs = world
            .queues
            .get(&queue_name(args.queue_id.as_str())?)
            .and_then(|q| q.job(&args.job_id))
            .map(|j| j.logs.clone())
            .unwrap_or_default();
        let total = logs.len() as u64;
        let lines: Vec<String> = logs
            .into_iter()
            .skip(args.offset as usize)
            .take(args.limit as usize)
            .collect();
        Ok(JobLogs {
            has_more: args.offset + (lines.len() as u64) < total,
            lines,
            total,
        })
    }

    async fn list_recent_jobs(&self, args: RecentJobsQuery) -> Result<RecentJobs, RanchError> {
        let world = self.world.lock().await;
        let ran = [JobState::Active, JobState::Completed, JobState::Failed];
        let jobs = world
            .queues
            .values()
            .filter(|q| {
                args.queue_id
                    .as_ref()
                    .is_none_or(|id| id.as_str() == q.id.as_str())
            })
            .flat_map(|q| {
                q.jobs
                    .iter()
                    .filter(|j| ran.contains(&j.state))
                    .map(|j| j.to_job(q))
            })
            .collect();
        Ok(RecentJobs {
            jobs: order_recent(jobs, clamp_limit(args.limit)),
            unreadable_queues: Vec::new(),
        })
    }

    async fn list_failed_jobs(&self, args: FailedJobsQuery) -> Result<FailedJobsPage, RanchError> {
        let world = self.world.lock().await;
        let candidates = Self::failed_candidates(&world);
        let filter = &args.filter;
        let (page, total) = page_of(&candidates, filter, args.offset, args.limit);
        let jobs = page
            .iter()
            .filter_map(|c| {
                let queue = world.queues.get(&c.queue_name)?;
                queue.job(&c.job_id).map(|j| j.to_job(queue))
            })
            .collect();
        let mut queues: Vec<FailedQueueCount> = world
            .queues
            .values()
            .map(|q| FailedQueueCount {
                queue_id: q.id.clone(),
                queue_name: q.name.clone(),
                failed: q.count(JobState::Failed) as u32,
            })
            .filter(|c| c.failed > 0)
            .collect();
        queues.sort_by_key(|c| std::cmp::Reverse(c.failed));
        Ok(FailedJobsPage {
            jobs,
            total,
            total_failed: candidates.len() as u32,
            queues,
            types: summarise_types(&candidates, filter.queue.as_deref()),
            scanned: candidates.len() as u32,
            scan_capped: false,
            unreadable_queues: Vec::new(),
        })
    }

    async fn list_failure_causes(
        &self,
        queue: Option<String>,
    ) -> Result<FailureCauses, RanchError> {
        let world = self.world.lock().await;
        let candidates = Self::failed_candidates(&world);
        let in_scope = candidates
            .iter()
            .filter(|c| queue.as_ref().is_none_or(|q| c.queue_name == *q))
            .count() as u32;
        Ok(FailureCauses {
            causes: group_by_cause(&candidates, queue.as_deref(), now_ms()),
            scanned: in_scope,
            scan_capped: false,
            total_failed: in_scope,
            unreadable_queues: Vec::new(),
        })
    }
}

#[async_trait]
impl JobActions for FakeBackend {
    async fn add_job(
        &self,
        queue_id: QueueId,
        job: NewJob,
        caller: Caller,
    ) -> Result<AddJobResult, RanchError> {
        let mut world = self.world.lock().await;
        let (queue_id, name) = queue_of(&queue_id)?;
        let refused = (AuditAction::AddJob, AuditTarget::queue(&queue_id, &name));
        self.authorize(&world, &caller, "add_job", refused).await?;
        let job =
            PreparedJob::try_from(&job).map_err(|reason| RanchError::InvalidInput { reason })?;
        let queue = world
            .queues
            .get_mut(&name)
            .ok_or_else(|| missing_queue(&name))?;
        let id = match &job.job_id {
            Some(id) if queue.job(id).is_some() => {
                return Err(RanchError::InvalidInput {
                    reason: format!(
                        "A job with ID \"{id}\" already exists in {name}: nothing was added"
                    ),
                })
            }
            Some(id) => id.clone(),
            None => {
                queue.last_id += 1;
                queue.last_id.to_string()
            }
        };
        let state = match job.placement() {
            Placement::Delayed => JobState::Delayed,
            Placement::Prioritized => states::PRIORITIZED,
            Placement::Standard => FakeJob::ready_state(queue.paused),
        };
        queue.jobs.push(FakeJob {
            name: job.name.clone(),
            data: job.data,
            ..FakeJob::new(&id, state, now_ms())
        });
        let target = AuditTarget::job(&queue_id, &name, &id);
        let reason = Some(format!("{} → {}", job.name, state.as_str()));
        self.record(&caller, AuditAction::AddJob, target, reason)
            .await?;
        Ok(AddJobResult { job_id: id, state })
    }

    async fn retry_job(
        &self,
        args: JobTarget,
        caller: Caller,
    ) -> Result<JobActionResult, RanchError> {
        let how = ("retry_job", "retry", AuditAction::RetryJob);
        self.move_one(args, caller, how, JobState::Failed).await
    }

    async fn remove_job(
        &self,
        args: JobTarget,
        caller: Caller,
    ) -> Result<JobActionResult, RanchError> {
        let mut world = self.world.lock().await;
        let (queue_id, name) = queue_of(&args.queue_id)?;
        let refused = (
            AuditAction::RemoveJob,
            AuditTarget::job(&queue_id, &name, &args.job_id),
        );
        self.authorize(&world, &caller, "remove_job", refused)
            .await?;
        let removed = world
            .queues
            .get_mut(&queue_name(args.queue_id.as_str())?)
            .and_then(|q| q.remove_job(&args.job_id));
        let (queue_id, name) = queue_of(&args.queue_id)?;
        if removed.is_none() {
            return Err(RanchError::JobNotFound {
                queue: name,
                id: args.job_id,
            });
        }
        let target = AuditTarget::job(&queue_id, &name, &args.job_id);
        self.record(&caller, AuditAction::RemoveJob, target, None)
            .await?;
        Ok(JobActionResult {
            job_id: args.job_id,
            new_state: None,
            action: "remove".to_string(),
        })
    }

    async fn promote_job(
        &self,
        args: JobTarget,
        caller: Caller,
    ) -> Result<JobActionResult, RanchError> {
        let how = ("promote_job", "promote", AuditAction::PromoteJob);
        self.move_one(args, caller, how, JobState::Delayed).await
    }

    async fn bulk_retry_jobs(
        &self,
        args: JobBatch,
        _progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<BulkActionResult, RanchError> {
        let how = ("bulk_retry_jobs", "retry", AuditAction::RetryJob);
        self.bulk(args, caller, how, retry_one).await
    }

    async fn bulk_remove_jobs(
        &self,
        args: JobBatch,
        _progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<BulkActionResult, RanchError> {
        let how = ("bulk_remove_jobs", "remove", AuditAction::RemoveJob);
        self.bulk(args, caller, how, remove_one).await
    }

    async fn bulk_promote_jobs(
        &self,
        args: JobBatch,
        _progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<BulkActionResult, RanchError> {
        let how = ("bulk_promote_jobs", "promote", AuditAction::PromoteJob);
        self.bulk(args, caller, how, promote_one).await
    }
}

/// Which jobs a maintenance operation acts on, at `now`.
fn targeted(op: &QueueMaintenance, job: &FakeJob, now: i64) -> bool {
    match op {
        QueueMaintenance::Drain { include_delayed } => match job.state {
            JobState::Waiting | JobState::Paused | states::PRIORITIZED => true,
            JobState::Delayed => *include_delayed,
            _ => false,
        },
        QueueMaintenance::Clean {
            state,
            older_than_ms,
        } => {
            let wanted = match state {
                CleanState::Completed => JobState::Completed,
                CleanState::Failed => JobState::Failed,
            };
            let cutoff = now.saturating_sub(i64::try_from(*older_than_ms).unwrap_or(i64::MAX));
            job.state == wanted && job.finished_on.unwrap_or(job.timestamp) <= cutoff
        }
        QueueMaintenance::RetryAllFailed => job.state == JobState::Failed,
    }
}

#[async_trait]
impl Maintenance for FakeBackend {
    async fn preview(
        &self,
        queue_id: QueueId,
        operation: QueueMaintenance,
    ) -> Result<MaintenancePreview, RanchError> {
        let world = self.world.lock().await;
        let now = now_ms();
        let Some(queue) = world.queues.get(&queue_name(queue_id.as_str())?) else {
            return Ok(MaintenancePreview {
                affected: 0,
                spared_scheduler_jobs: 0,
            });
        };
        let jobs = queue.jobs.iter().filter(|j| targeted(&operation, j, now));
        let (spared, affected): (Vec<&FakeJob>, Vec<&FakeJob>) = jobs.partition(|j| {
            matches!(operation, QueueMaintenance::Drain { .. }) && j.scheduler.is_some()
        });
        Ok(MaintenancePreview {
            affected: affected.len() as u64,
            spared_scheduler_jobs: spared.len() as u64,
        })
    }

    /// Flow members and schedulers' pending runs are left in place and
    /// reported as skipped.
    async fn run(
        &self,
        queue_id: QueueId,
        operation: QueueMaintenance,
        _progress: Arc<dyn ProgressSink>,
        caller: Caller,
    ) -> Result<MaintenanceResult, RanchError> {
        let op = &operation;
        let mut world = self.world.lock().await;
        let (queue_id, name) = queue_of(&queue_id)?;
        let action = match op {
            QueueMaintenance::Drain { .. } => AuditAction::DrainQueue,
            QueueMaintenance::Clean { .. } => AuditAction::CleanQueue,
            QueueMaintenance::RetryAllFailed => AuditAction::RetryAllFailed,
        };
        let refused = (action, AuditTarget::queue(&queue_id, &name));
        self.authorize(&world, &caller, op.tool(), refused).await?;
        let now = now_ms();
        let mut undeclared = FakeQueue::new(&name);
        let queue = world.queues.get_mut(&name).unwrap_or(&mut undeclared);
        let ids: Vec<String> = queue
            .jobs
            .iter()
            .filter(|j| targeted(op, j, now))
            .map(|j| j.id.clone())
            .collect();
        let (mut affected, mut skipped) = (0u64, 0u64);
        for id in ids {
            let job = queue.job(&id).expect("listed above");
            if job.scheduler.is_some() || queue.is_flow_member(job) {
                skipped += 1;
                continue;
            }
            match op {
                QueueMaintenance::RetryAllFailed => {
                    retry_one(queue, &id).expect("a failed job");
                }
                _ => {
                    queue.remove_job(&id);
                }
            }
            affected += 1;
        }
        let verb = match op {
            QueueMaintenance::Drain { .. } | QueueMaintenance::Clean { .. } => "removed",
            QueueMaintenance::RetryAllFailed => "retried",
        };
        let mut reason = format!("{affected} {verb}");
        if skipped > 0 {
            reason.push_str(&format!(", {skipped} left in place"));
        }
        let target = AuditTarget::queue(&queue_id, &name);
        self.record(&caller, action, target, Some(reason)).await?;
        Ok(MaintenanceResult {
            action: op.verb().to_string(),
            affected,
            skipped,
            cancelled: false,
        })
    }
}

#[async_trait]
impl QueueControl for FakeBackend {
    /// Pausing parks the waiting jobs, as BullMQ moves `wait` to `paused`.
    async fn pause_queue(
        &self,
        queue_id: QueueId,
        caller: Caller,
    ) -> Result<QueueActionResult, RanchError> {
        let (queue_id, name) = queue_of(&queue_id)?;
        let mut world = self.world.lock().await;
        let refused = (
            AuditAction::PauseQueue,
            AuditTarget::queue(&queue_id, &name),
        );
        self.authorize(&world, &caller, "pause_queue", refused)
            .await?;
        let queue = queue_entry(&mut world, &name);
        let already_in_state = queue.paused;
        queue.paused = true;
        for job in queue
            .jobs
            .iter_mut()
            .filter(|j| j.state == JobState::Waiting)
        {
            job.state = JobState::Paused;
        }
        let target = AuditTarget::queue(&queue_id, &name);
        let reason = already_in_state.then(|| "already paused".to_string());
        self.record(&caller, AuditAction::PauseQueue, target, reason)
            .await?;
        Ok(QueueActionResult {
            is_paused: true,
            already_in_state,
        })
    }

    async fn resume_queue(
        &self,
        queue_id: QueueId,
        caller: Caller,
    ) -> Result<QueueActionResult, RanchError> {
        let (queue_id, name) = queue_of(&queue_id)?;
        let mut world = self.world.lock().await;
        let refused = (
            AuditAction::ResumeQueue,
            AuditTarget::queue(&queue_id, &name),
        );
        self.authorize(&world, &caller, "resume_queue", refused)
            .await?;
        let queue = queue_entry(&mut world, &name);
        let already_in_state = !queue.paused;
        queue.paused = false;
        for job in queue
            .jobs
            .iter_mut()
            .filter(|j| j.state == JobState::Paused)
        {
            job.state = JobState::Waiting;
        }
        let target = AuditTarget::queue(&queue_id, &name);
        let reason = already_in_state.then(|| "already running".to_string());
        self.record(&caller, AuditAction::ResumeQueue, target, reason)
            .await?;
        Ok(QueueActionResult {
            is_paused: false,
            already_in_state,
        })
    }
}

impl FakeBackend {
    fn scheduler_not_found(&self, args: &SchedulerTarget) -> RanchError {
        RanchError::SchedulerNotFound {
            connection_id: self.stamp.id.clone(),
            queue_name: queue_name(args.queue_id.as_str()).unwrap_or_default(),
            scheduler_key: args.scheduler_key.clone(),
        }
    }
}

fn scheduler_target(args: &SchedulerTarget) -> Result<AuditTarget, RanchError> {
    let (queue_id, name) = queue_of(&args.queue_id)?;
    Ok(AuditTarget::scheduler(
        &queue_id,
        &name,
        &args.scheduler_key,
    ))
}

/// When a scheduler runs next, from now: a pattern's next time, or now for
/// an interval (its first run is due at once).
fn next_run(schedule: &Schedule, now: chrono::DateTime<chrono::Utc>) -> i64 {
    match schedule {
        Schedule::Pattern { .. } => schedule
            .next_runs(now, 1)
            .ok()
            .and_then(|runs| runs.first().copied())
            .unwrap_or(now.timestamp_millis()),
        Schedule::Every { .. } => now.timestamp_millis(),
    }
}

#[async_trait]
impl Schedules for FakeBackend {
    async fn list_schedulers(&self, queue_id: QueueId) -> Result<Vec<CronJob>, RanchError> {
        let world = self.world.lock().await;
        Ok(world
            .queues
            .get(&queue_name(queue_id.as_str())?)
            .map(|q| {
                q.schedulers
                    .iter()
                    .map(FakeScheduler::to_cron_job)
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn upsert_scheduler(
        &self,
        queue_id: QueueId,
        spec: SchedulerSpec,
        caller: Caller,
    ) -> Result<SchedulerSaved, RanchError> {
        let mut world = self.world.lock().await;
        let refused = {
            let (queue_id, name) = queue_of(&queue_id)?;
            let target = AuditTarget::scheduler(&queue_id, &name, &spec.id);
            (AuditAction::UpsertScheduler, target)
        };
        self.authorize(&world, &caller, "upsert_scheduler", refused)
            .await?;
        let now = chrono::Utc::now();
        let prepared = spec
            .prepare(now)
            .map_err(|reason| RanchError::InvalidInput { reason })?;
        let (queue_id, name) = queue_of(&queue_id)?;
        let queue = world
            .queues
            .get_mut(&name)
            .ok_or_else(|| missing_queue(&name))?;
        let schedule = match (&prepared.pattern, prepared.every_ms) {
            (Some(pattern), _) => Schedule::Pattern {
                pattern: pattern.clone(),
                tz: prepared.tz.clone(),
            },
            (None, every_ms) => Schedule::Every {
                every_ms: every_ms.unwrap_or_default(),
            },
        };
        queue.disarm(&prepared.id);
        queue.schedulers.retain(|s| s.id != prepared.id);
        let next = prepared.first_run_ms.unwrap_or(now.timestamp_millis());
        queue.schedulers.push(FakeScheduler {
            id: prepared.id.clone(),
            job_name: prepared.job_name.clone(),
            schedule,
            data: prepared.data.clone(),
            paused: false,
            next_run: next,
        });
        let job_id = queue
            .arm(&prepared.id, now.timestamp_millis())
            .expect("pushed above");
        let target = AuditTarget::scheduler(&queue_id, &name, &prepared.id);
        self.record(&caller, AuditAction::UpsertScheduler, target, None)
            .await?;
        Ok(SchedulerSaved {
            scheduler_id: prepared.id,
            next_run_ms: next,
            job_id,
        })
    }

    async fn remove_scheduler(
        &self,
        args: SchedulerTarget,
        caller: Caller,
    ) -> Result<SchedulerActionResult, RanchError> {
        let mut world = self.world.lock().await;
        let refused = (AuditAction::RemoveScheduler, scheduler_target(&args)?);
        self.authorize(&world, &caller, "remove_scheduler", refused)
            .await?;
        let queue = world.queues.get_mut(&queue_name(args.queue_id.as_str())?);
        let Some(queue) = queue.filter(|q| q.scheduler(&args.scheduler_key).is_some()) else {
            return Err(self.scheduler_not_found(&args));
        };
        queue.disarm(&args.scheduler_key);
        queue.schedulers.retain(|s| s.id != args.scheduler_key);
        self.record(
            &caller,
            AuditAction::RemoveScheduler,
            scheduler_target(&args)?,
            None,
        )
        .await?;
        Ok(SchedulerActionResult {
            paused: false,
            already_in_state: false,
        })
    }

    /// A paused scheduler keeps its settings and loses its pending run.
    async fn pause_scheduler(
        &self,
        args: SchedulerTarget,
        caller: Caller,
    ) -> Result<SchedulerActionResult, RanchError> {
        let mut world = self.world.lock().await;
        let refused = (AuditAction::PauseScheduler, scheduler_target(&args)?);
        self.authorize(&world, &caller, "pause_scheduler", refused)
            .await?;
        let Some(queue) = world.queues.get_mut(&queue_name(args.queue_id.as_str())?) else {
            return Err(self.scheduler_not_found(&args));
        };
        let Some(scheduler) = queue
            .schedulers
            .iter_mut()
            .find(|s| s.id == args.scheduler_key)
        else {
            return Err(self.scheduler_not_found(&args));
        };
        let already_in_state = scheduler.paused;
        scheduler.paused = true;
        queue.disarm(&args.scheduler_key);
        let reason = already_in_state.then(|| "already paused".to_string());
        self.record(
            &caller,
            AuditAction::PauseScheduler,
            scheduler_target(&args)?,
            reason,
        )
        .await?;
        Ok(SchedulerActionResult {
            paused: true,
            already_in_state,
        })
    }

    /// Resuming re-arms the scheduler from now.
    async fn resume_scheduler(
        &self,
        args: SchedulerTarget,
        caller: Caller,
    ) -> Result<SchedulerActionResult, RanchError> {
        let mut world = self.world.lock().await;
        let refused = (AuditAction::ResumeScheduler, scheduler_target(&args)?);
        self.authorize(&world, &caller, "resume_scheduler", refused)
            .await?;
        let now = chrono::Utc::now();
        let Some(queue) = world.queues.get_mut(&queue_name(args.queue_id.as_str())?) else {
            return Err(self.scheduler_not_found(&args));
        };
        let Some(scheduler) = queue
            .schedulers
            .iter_mut()
            .find(|s| s.id == args.scheduler_key)
        else {
            return Err(self.scheduler_not_found(&args));
        };
        let already_in_state = !scheduler.paused;
        if !already_in_state {
            scheduler.paused = false;
            scheduler.next_run = next_run(&scheduler.schedule, now);
            queue.arm(&args.scheduler_key, now.timestamp_millis());
        }
        let reason = already_in_state.then(|| "already active".to_string());
        self.record(
            &caller,
            AuditAction::ResumeScheduler,
            scheduler_target(&args)?,
            reason,
        )
        .await?;
        Ok(SchedulerActionResult {
            paused: false,
            already_in_state,
        })
    }
}

#[async_trait]
impl Workers for FakeBackend {
    async fn list_workers(&self, queue_id: QueueId) -> Result<QueueWorkers, RanchError> {
        let world = self.world.lock().await;
        let names = world
            .queues
            .get(&queue_name(queue_id.as_str())?)
            .map(|q| q.workers.clone())
            .unwrap_or_default();
        Ok(QueueWorkers {
            supported: true,
            unsupported_reason: None,
            workers: names
                .into_iter()
                .enumerate()
                .map(|(i, name)| QueueWorker {
                    id: (i + 1).to_string(),
                    name: Some(name),
                    addr: "in-memory".to_string(),
                    age_secs: 0,
                    idle_secs: 0,
                    last_command: None,
                    library: None,
                    raw: Default::default(),
                })
                .collect(),
        })
    }

    async fn count_workers(
        &self,
        queue_ids: Vec<QueueId>,
    ) -> Result<QueueWorkerCounts, RanchError> {
        let world = self.world.lock().await;
        Ok(QueueWorkerCounts {
            supported: true,
            counts: queue_ids
                .into_iter()
                .map(|queue_id| {
                    let workers = queue_name(queue_id.as_str())
                        .ok()
                        .and_then(|name| world.queues.get(&name))
                        .map_or(0, |q| q.workers.len() as u32);
                    QueueWorkerCount { queue_id, workers }
                })
                .collect(),
        })
    }
}

#[async_trait]
impl Flows for FakeBackend {
    async fn get_flow(&self, args: JobTarget) -> Result<FlowTree, RanchError> {
        let world = self.world.lock().await;
        let name = queue_name(args.queue_id.as_str())?;
        let not_found = || RanchError::JobNotFound {
            queue: name.clone(),
            id: args.job_id.clone(),
        };
        let queue = world.queues.get(&name).ok_or_else(not_found)?;
        let mut root = queue.job(&args.job_id).ok_or_else(not_found)?;
        while let Some(parent) = root.parent.as_deref().and_then(|p| queue.job(p)) {
            root = parent;
        }
        // The root, then each generation below it.
        let mut members = vec![root];
        let mut i = 0;
        while i < members.len() {
            let id = members[i].id.clone();
            members.extend(queue.jobs.iter().filter(|j| j.parent.as_ref() == Some(&id)));
            i += 1;
        }
        let q = queue;
        let nodes: Vec<FlowNode> = members
            .iter()
            .map(|j| FlowNode {
                key: job_key(q, &j.id),
                job_ref: j.to_job(q).job_ref,
                parent_key: j.parent.as_ref().map(|p| job_key(q, p)),
                name: j.name.clone(),
                state: j.state,
                timestamp: Some(j.timestamp),
                duration_ms: None,
                failed_reason: j.failed_reason.clone(),
                missing: false,
                return_value: None,
                blocks_parent: false,
            })
            .collect();
        let children: Vec<(JobState, bool)> = members
            .iter()
            .filter(|j| j.parent.as_ref() == Some(&root.id))
            .map(|j| (j.state, false))
            .collect();
        Ok(FlowTree {
            nodes,
            root_key: job_key(q, &root.id),
            focus_key: job_key(q, &args.job_id),
            truncated: false,
            blocked: is_blocked(root.state, &children),
        })
    }

    async fn list_flow_roots(&self, queue_id: QueueId) -> Result<FlowRoots, RanchError> {
        let world = self.world.lock().await;
        let Some(queue) = world.queues.get(&queue_name(queue_id.as_str())?) else {
            return Ok(FlowRoots {
                roots: Vec::new(),
                scanned: 0,
                capped: false,
            });
        };
        let roots = queue
            .jobs
            .iter()
            .filter(|j| j.parent.is_none())
            .filter_map(|root| {
                let children: Vec<&FakeJob> = queue
                    .jobs
                    .iter()
                    .filter(|j| j.parent.as_ref() == Some(&root.id))
                    .collect();
                if children.is_empty() {
                    return None;
                }
                let count = |state: JobState| children.iter().filter(|c| c.state == state).count();
                let completed = count(JobState::Completed);
                let failed = count(JobState::Failed);
                let states: Vec<(JobState, bool)> =
                    children.iter().map(|c| (c.state, false)).collect();
                Some(FlowRoot {
                    job_ref: root.to_job(queue).job_ref,
                    name: root.name.clone(),
                    state: root.state,
                    timestamp: Some(root.timestamp),
                    children_total: children.len() as u64,
                    children_pending: (children.len() - completed - failed) as u64,
                    children_completed: completed as u64,
                    children_failed: failed as u64,
                    blocked: is_blocked(root.state, &states),
                })
            })
            .collect();
        Ok(FlowRoots {
            roots,
            scanned: queue.jobs.len() as u64,
            capped: false,
        })
    }
}
