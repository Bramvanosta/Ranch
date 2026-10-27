//! The summary of a window dragged on the Timeline:
//! what finished in it (done, failed), how long jobs waited (p95) against
//! the rest of the visible range, and its failures grouped by cause.
//!
//! The actor reads the jobs that finished in the visible range (newest
//! first, capped per queue and state — the completed / failed sets are
//! scored by finish time) and hands them to [`summarize`], which is pure.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::domain::JobRef;
use super::failed::failure_signature;

/// Newest finished jobs read per queue and state.
pub const WINDOW_SCAN_PER_SET: usize = 2_000;

/// A job that finished in the visible range.
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedJob {
    pub job_ref: JobRef,
    pub failed: bool,
    pub timestamp: i64,
    pub processed_on: Option<i64>,
    pub finished_on: i64,
    pub failed_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowCause {
    pub signature: String,
    pub sample: String,
    pub count: u32,
    /// The jobs of the cause in the window, newest first ("Open N jobs").
    pub jobs: Vec<JobRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowSummary {
    pub from_ms: i64,
    pub to_ms: i64,
    pub done: u32,
    pub failed: u32,
    /// What the rest of the visible range did, scaled to the window's length.
    pub usual_done: f64,
    pub usual_failed: f64,
    /// Time from added to started, 95th percentile; `None` = no job started.
    pub wait_p95_ms: Option<i64>,
    pub usual_wait_p95_ms: Option<i64>,
    /// Most frequent first.
    pub causes: Vec<WindowCause>,
    /// Jobs looked at, and whether older ones were left out.
    pub scanned: u32,
    pub scan_capped: bool,
}

/// 95th percentile (nearest rank).
pub fn p95(values: &mut [i64]) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let rank = ((values.len() as f64) * 0.95).ceil() as usize;
    Some(values[rank.clamp(1, values.len()) - 1])
}

/// `jobs` finished in `[range_from, now]`; the window is `[from, to]`.
pub fn summarize(
    jobs: &[FinishedJob],
    from: i64,
    to: i64,
    range_from: i64,
    now: i64,
    scan_capped: bool,
) -> WindowSummary {
    let in_window = |j: &FinishedJob| j.finished_on >= from && j.finished_on <= to;
    let in_range = |j: &FinishedJob| j.finished_on >= range_from && j.finished_on <= now;
    let wait = |j: &FinishedJob| j.processed_on.map(|p| (p - j.timestamp).max(0));

    let window: Vec<&FinishedJob> = jobs.iter().filter(|j| in_window(j)).collect();
    let usual: Vec<&FinishedJob> = jobs
        .iter()
        .filter(|j| in_range(j) && !in_window(j))
        .collect();

    let window_len = (to - from).max(1) as f64;
    let usual_len = ((now - range_from) - (to - from)).max(1) as f64;
    let scale = window_len / usual_len;
    let count =
        |set: &[&FinishedJob], failed: bool| set.iter().filter(|j| j.failed == failed).count();

    let mut window_waits: Vec<i64> = window.iter().filter_map(|j| wait(j)).collect();
    let mut usual_waits: Vec<i64> = usual.iter().filter_map(|j| wait(j)).collect();

    // Failures of the window by cause, newest first inside each cause.
    let mut failures: Vec<&&FinishedJob> = window.iter().filter(|j| j.failed).collect();
    failures.sort_by_key(|j| std::cmp::Reverse(j.finished_on));
    let mut causes: Vec<WindowCause> = Vec::new();
    for job in failures {
        let reason = job.failed_reason.clone().unwrap_or_default();
        let signature = failure_signature(&reason);
        match causes.iter_mut().find(|c| c.signature == signature) {
            Some(cause) => {
                cause.count += 1;
                cause.jobs.push(job.job_ref.clone());
            }
            None => causes.push(WindowCause {
                signature,
                sample: reason,
                count: 1,
                jobs: vec![job.job_ref.clone()],
            }),
        }
    }
    causes.sort_by_key(|c| std::cmp::Reverse(c.count));

    WindowSummary {
        from_ms: from,
        to_ms: to,
        done: count(&window, false) as u32,
        failed: count(&window, true) as u32,
        usual_done: count(&usual, false) as f64 * scale,
        usual_failed: count(&usual, true) as f64 * scale,
        wait_p95_ms: p95(&mut window_waits),
        usual_wait_p95_ms: p95(&mut usual_waits),
        causes,
        scanned: jobs.len() as u32,
        scan_capped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::bullmq_schema::QueueRef;

    fn job(
        id: &str,
        failed: bool,
        added: i64,
        started: i64,
        finished: i64,
        reason: &str,
    ) -> FinishedJob {
        FinishedJob {
            job_ref: JobRef {
                queue: QueueRef {
                    prefix: "bull".into(),
                    name: "emails".into(),
                },
                id: id.into(),
            },
            failed,
            timestamp: added,
            processed_on: Some(started),
            finished_on: finished,
            failed_reason: (!reason.is_empty()).then(|| reason.to_string()),
        }
    }

    #[test]
    fn p95_is_the_nearest_rank() {
        let mut v: Vec<i64> = (1..=100).collect();
        assert_eq!(p95(&mut v), Some(95));
        assert_eq!(p95(&mut [7]), Some(7));
        assert_eq!(p95(&mut []), None);
    }

    #[test]
    fn a_window_is_compared_with_the_rest_of_the_range() {
        // Range [0, 1000]; window [400, 500] (a tenth of the range).
        let jobs = vec![
            job("1", false, 395, 400, 410, ""),
            job("2", true, 300, 450, 460, "connect ECONNREFUSED 10.0.0.1:25"),
            job("3", true, 300, 470, 480, "connect ECONNREFUSED 10.0.0.2:25"),
            job("4", true, 300, 480, 490, "Invalid email \"x\""),
            job("5", false, 100, 110, 120, ""),
            job("6", false, 600, 610, 700, ""),
            job("7", true, 800, 805, 900, "boom"),
        ];
        let s = summarize(&jobs, 400, 500, 0, 1000, false);
        assert_eq!((s.done, s.failed), (1, 3));
        // Outside the window: 2 done and 1 failed over 900 ms → ×100/900.
        assert!((s.usual_done - 2.0 * 100.0 / 900.0).abs() < 1e-9);
        assert!((s.usual_failed - 100.0 / 900.0).abs() < 1e-9);
        assert_eq!(s.wait_p95_ms, Some(180));
        assert_eq!(s.usual_wait_p95_ms, Some(10));
        assert_eq!(s.causes.len(), 2);
        assert_eq!(s.causes[0].count, 2, "same cause, other values");
        assert_eq!(s.causes[0].jobs[0].id, "3", "newest first");
        assert_eq!(s.scanned, 7);
    }
}
