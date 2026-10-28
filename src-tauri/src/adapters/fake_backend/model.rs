//! The fake's world: queues of jobs, schedulers and workers, kept in
//! memory, and the conversions to the contract's types.

use std::collections::BTreeMap;

use crate::adapters::bullmq_redis::bullmq_schema::states;
use crate::adapters::bullmq_redis::bullmq_schema::{CronJob, Job, JobRef, JobState};
use crate::domain::errors::RanchError;
use crate::domain::failures::FailedCandidate;
use crate::domain::lock::ReadOnlyState;
use crate::domain::queues::{Queue, QueueId};
use crate::domain::schedule::Schedule;

/// One connection's queues and its live lock state.
pub(super) struct World {
    pub lock: ReadOnlyState,
    /// By queue name.
    pub queues: BTreeMap<String, FakeQueue>,
}

pub(super) struct FakeQueue {
    pub id: QueueId,
    pub name: String,
    pub paused: bool,
    /// Oldest first.
    pub jobs: Vec<FakeJob>,
    pub schedulers: Vec<FakeScheduler>,
    /// Names of the workers consuming the queue.
    pub workers: Vec<String>,
    /// The last id given to a job added without one.
    pub last_id: u64,
}

/// A job as tests seed it and the fake keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeJob {
    pub id: String,
    pub name: String,
    pub data: String,
    pub state: JobState,
    pub timestamp: i64,
    pub finished_on: Option<i64>,
    pub failed_reason: Option<String>,
    /// The id of its parent, in the same queue, when it is a flow's child.
    pub parent: Option<String>,
    pub logs: Vec<String>,
    /// The scheduler whose next run this job is.
    pub scheduler: Option<String>,
}

pub(super) struct FakeScheduler {
    pub id: String,
    pub job_name: String,
    pub schedule: Schedule,
    pub data: String,
    pub paused: bool,
    pub next_run: i64,
}

impl FakeJob {
    /// A job named `test-job`, queued at `at`. Finished states carry `at`
    /// as their end; a failed job carries `kaboom` as its reason.
    pub fn new(id: &str, state: JobState, at: i64) -> Self {
        let finished = matches!(state, JobState::Completed | JobState::Failed);
        FakeJob {
            id: id.to_string(),
            name: "test-job".to_string(),
            data: "{}".to_string(),
            state,
            timestamp: at,
            finished_on: finished.then_some(at),
            failed_reason: (state == JobState::Failed).then(|| "kaboom".to_string()),
            parent: None,
            logs: Vec::new(),
            scheduler: None,
        }
    }

    pub fn child_of(mut self, parent: &str) -> Self {
        self.parent = Some(parent.to_string());
        self
    }

    pub(super) fn to_job(&self, queue: &FakeQueue) -> Job {
        Job {
            job_ref: queue.job_ref(&self.id),
            name: self.name.clone(),
            state: self.state,
            data: self.data.clone(),
            opts: "{}".to_string(),
            attempts_made: 0,
            timestamp: self.timestamp,
            processed_on: None,
            finished_on: self.finished_on,
            delay: None,
            failed_reason: self.failed_reason.clone(),
            stacktrace: Vec::new(),
            return_value: None,
            parent_key: self.parent.as_ref().map(|p| job_key(queue, p)),
            progress: None,
            processed_by: None,
            lock_ttl_ms: None,
        }
    }

    /// Where it would go back to, or be put, on a queue paused or not.
    pub(super) fn ready_state(queue_paused: bool) -> JobState {
        if queue_paused {
            JobState::Paused
        } else {
            JobState::Waiting
        }
    }
}

impl FakeQueue {
    pub fn new(name: &str) -> Self {
        FakeQueue {
            id: queue_id(name),
            name: name.to_string(),
            paused: false,
            jobs: Vec::new(),
            schedulers: Vec::new(),
            workers: Vec::new(),
            last_id: 0,
        }
    }

    pub fn job_ref(&self, id: &str) -> JobRef {
        JobRef {
            queue_id: self.id.clone(),
            queue_name: self.name.clone(),
            id: id.to_string(),
        }
    }

    pub fn job(&self, id: &str) -> Option<&FakeJob> {
        self.jobs.iter().find(|j| j.id == id)
    }

    pub fn job_mut(&mut self, id: &str) -> Option<&mut FakeJob> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    pub fn remove_job(&mut self, id: &str) -> Option<FakeJob> {
        let at = self.jobs.iter().position(|j| j.id == id)?;
        Some(self.jobs.remove(at))
    }

    pub fn count(&self, state: JobState) -> u64 {
        self.jobs.iter().filter(|j| j.state == state).count() as u64
    }

    pub fn is_flow_member(&self, job: &FakeJob) -> bool {
        job.parent.is_some() || self.jobs.iter().any(|j| j.parent.as_ref() == Some(&job.id))
    }

    pub fn to_queue(&self) -> Queue {
        Queue {
            id: self.id.clone(),
            name: self.name.clone(),
            counts: [
                JobState::Waiting,
                JobState::Active,
                JobState::Completed,
                JobState::Failed,
                JobState::Delayed,
                states::PRIORITIZED,
                JobState::Paused,
            ]
            .into_iter()
            .map(|state| (state, self.count(state)))
            .collect(),
            is_paused: self.paused,
        }
    }

    pub fn failed_candidates(&self) -> impl Iterator<Item = FailedCandidate> + '_ {
        self.jobs
            .iter()
            .filter(|j| j.state == JobState::Failed)
            .map(|j| FailedCandidate {
                queue_id: self.id.clone(),
                queue_name: self.name.clone(),
                job_id: j.id.clone(),
                failed_at: j.finished_on.unwrap_or(j.timestamp),
                name: j.name.clone(),
                reason: j.failed_reason.clone(),
            })
    }

    pub fn scheduler(&self, id: &str) -> Option<&FakeScheduler> {
        self.schedulers.iter().find(|s| s.id == id)
    }

    /// Queue the scheduler's next run as a job, as BullMQ does: due now
    /// (an interval's first run) it waits, later it is delayed.
    pub fn arm(&mut self, scheduler_id: &str, now: i64) -> Option<String> {
        let (job_name, data, next_run) = {
            let s = self.scheduler(scheduler_id)?;
            (s.job_name.clone(), s.data.clone(), s.next_run)
        };
        let id = format!("repeat:{scheduler_id}:{next_run}");
        let state = if next_run <= now {
            FakeJob::ready_state(self.paused)
        } else {
            JobState::Delayed
        };
        self.jobs.push(FakeJob {
            id: id.clone(),
            name: job_name,
            data,
            state,
            timestamp: now,
            finished_on: None,
            failed_reason: None,
            parent: None,
            logs: Vec::new(),
            scheduler: Some(scheduler_id.to_string()),
        });
        Some(id)
    }

    /// Drop the scheduler's pending run, if any.
    pub fn disarm(&mut self, scheduler_id: &str) {
        self.jobs.retain(|j| {
            j.scheduler.as_deref() != Some(scheduler_id)
                || !matches!(
                    j.state,
                    JobState::Waiting | JobState::Paused | JobState::Delayed
                )
        });
    }
}

impl FakeScheduler {
    pub fn to_cron_job(&self) -> CronJob {
        let (pattern, every, tz) = match &self.schedule {
            Schedule::Pattern { pattern, tz } => (Some(pattern.clone()), None, tz.clone()),
            Schedule::Every { every_ms } => (None, Some(*every_ms), None),
        };
        CronJob {
            key: self.id.clone(),
            name: self.job_name.clone(),
            pattern,
            every,
            tz,
            offset: None,
            iteration_count: 0,
            next_run: self.next_run,
            end_date: None,
            template: Some(self.data.clone()),
            paused: self.paused,
        }
    }
}

/// The fake's own queue ids, `memory/{name}`: unlike any other backend's,
/// so a caller that builds an id instead of taking it from the listing
/// does not find the queue.
pub(super) fn queue_id(name: &str) -> QueueId {
    QueueId::new(format!("memory/{name}"))
}

/// The name in an id [`queue_id`] made.
pub(super) fn queue_name(id: &str) -> Result<String, RanchError> {
    id.strip_prefix("memory/")
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .ok_or_else(|| RanchError::InvalidInput {
            reason: format!("'{id}' is not a queue id: take one from the queue list"),
        })
}

/// A job's key as flows name it: `{queue id}/{job id}`.
pub(super) fn job_key(queue: &FakeQueue, id: &str) -> String {
    format!("{}/{id}", queue.id)
}
