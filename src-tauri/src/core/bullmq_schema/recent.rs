//! Recent activity — the jobs that ran last, on a queue or across the queues
//! of a connection.
//!
//! Pure logic: which finished jobs are worth fetching, and in what order the
//! result is shown. The actor fetches; this decides. Unit-tested without Redis.
//!
//! The cost is bounded whatever the size of the queues: the newest `limit`
//! entries of each `completed` / `failed` set and the head of each `active`
//! list are read (ids and scores only), and full hashes are fetched for at
//! most `limit` finished jobs plus [`MAX_ACTIVE`] running ones.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::domain::{Job, JobState, QueueRef};

pub const DEFAULT_LIMIT: u32 = 15;
pub const MAX_LIMIT: u32 = 50;
/// Running jobs shown at most (they come first: they are the most recent).
pub const MAX_ACTIVE: usize = 25;

/// A finished job as the scan sees it: where it is and when it ended.
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedCandidate {
    pub queue: QueueRef,
    pub job_id: String,
    pub state: JobState,
    /// Score in the `completed` / `failed` sorted set = `finishedOn` (ms).
    pub finished_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecentJobs {
    /// Running jobs first (latest started first), then finished ones, latest first.
    pub jobs: Vec<Job>,
    /// Queues whose sets could not be read — reported, never fatal.
    pub unreadable_queues: Vec<String>,
}

pub fn clamp_limit(limit: u32) -> usize {
    (if limit == 0 { DEFAULT_LIMIT } else { limit }).min(MAX_LIMIT) as usize
}

/// The `limit` most recently finished candidates, latest first. Ties (same
/// millisecond) are ordered by queue then id so the list does not shuffle
/// between two refreshes.
pub fn newest_finished(mut all: Vec<FinishedCandidate>, limit: usize) -> Vec<FinishedCandidate> {
    all.sort_by(|a, b| {
        b.finished_at
            .cmp(&a.finished_at)
            .then_with(|| a.queue.name.cmp(&b.queue.name))
            .then_with(|| a.job_id.cmp(&b.job_id))
    });
    all.truncate(limit);
    all
}

/// When a job last did something: it ended, or it started, or it was added.
pub fn last_activity(job: &Job) -> i64 {
    job.finished_on
        .or(job.processed_on)
        .unwrap_or(job.timestamp)
}

/// Final order: running jobs first, then by last activity, latest first;
/// `limit` jobs in total.
pub fn order_recent(mut jobs: Vec<Job>, limit: usize) -> Vec<Job> {
    jobs.sort_by(|a, b| {
        let running = |j: &Job| j.state == JobState::Active;
        running(b)
            .cmp(&running(a))
            .then_with(|| last_activity(b).cmp(&last_activity(a)))
            .then_with(|| a.job_ref.queue.name.cmp(&b.job_ref.queue.name))
            .then_with(|| a.job_ref.id.cmp(&b.job_ref.id))
    });
    jobs.truncate(limit);
    jobs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::bullmq_schema::JobRef;

    fn q(name: &str) -> QueueRef {
        QueueRef {
            prefix: "bull".into(),
            name: name.into(),
        }
    }

    fn cand(queue: &str, id: &str, state: JobState, at: i64) -> FinishedCandidate {
        FinishedCandidate {
            queue: q(queue),
            job_id: id.into(),
            state,
            finished_at: at,
        }
    }

    fn job(
        queue: &str,
        id: &str,
        state: JobState,
        processed: Option<i64>,
        finished: Option<i64>,
    ) -> Job {
        Job {
            job_ref: JobRef {
                queue: q(queue),
                id: id.into(),
            },
            name: "n".into(),
            state,
            data: "{}".into(),
            opts: "{}".into(),
            attempts_made: 0,
            timestamp: 1,
            processed_on: processed,
            finished_on: finished,
            delay: None,
            failed_reason: None,
            stacktrace: Vec::new(),
            return_value: None,
            parent_key: None,
            progress: None,
            processed_by: None,
            lock_ttl_ms: None,
        }
    }

    #[test]
    fn a_limit_of_zero_means_the_default_and_a_huge_one_is_capped() {
        assert_eq!(clamp_limit(0), DEFAULT_LIMIT as usize);
        assert_eq!(clamp_limit(10), 10);
        assert_eq!(clamp_limit(10_000), MAX_LIMIT as usize);
    }

    #[test]
    fn the_newest_finished_jobs_win_whatever_their_queue_or_outcome() {
        let picked = newest_finished(
            vec![
                cand("emails", "1", JobState::Completed, 100),
                cand("emails", "2", JobState::Failed, 400),
                cand("reports", "9", JobState::Completed, 300),
                cand("reports", "8", JobState::Completed, 50),
            ],
            3,
        );
        let ids: Vec<&str> = picked.iter().map(|c| c.job_id.as_str()).collect();
        assert_eq!(ids, ["2", "9", "1"]);
    }

    #[test]
    fn ties_keep_a_stable_order() {
        let a = newest_finished(
            vec![
                cand("reports", "2", JobState::Completed, 100),
                cand("emails", "7", JobState::Completed, 100),
                cand("emails", "3", JobState::Completed, 100),
            ],
            10,
        );
        let ids: Vec<(&str, &str)> = a
            .iter()
            .map(|c| (c.queue.name.as_str(), c.job_id.as_str()))
            .collect();
        assert_eq!(ids, [("emails", "3"), ("emails", "7"), ("reports", "2")]);
    }

    #[test]
    fn running_jobs_come_first_then_the_latest_activity() {
        let ordered = order_recent(
            vec![
                job(
                    "emails",
                    "done-late",
                    JobState::Completed,
                    Some(10),
                    Some(900),
                ),
                job("emails", "running-old", JobState::Active, Some(100), None),
                job("emails", "failed", JobState::Failed, Some(10), Some(500)),
                job("emails", "running-new", JobState::Active, Some(800), None),
            ],
            3,
        );
        let ids: Vec<&str> = ordered.iter().map(|j| j.job_ref.id.as_str()).collect();
        assert_eq!(ids, ["running-new", "running-old", "done-late"]);
    }

    #[test]
    fn a_job_that_never_started_counts_from_when_it_was_added() {
        let j = job("emails", "1", JobState::Active, None, None);
        assert_eq!(last_activity(&j), 1);
    }
}
