//! Operations on a whole queue: what they would touch and what they did.

/// What a maintenance operation would touch, shown before confirming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenancePreview {
    /// Jobs the operation will act on. For drain this is an upper bound:
    /// flow members are only recognised job by job, while running.
    pub affected: u64,
    /// Drain only: pending scheduler runs that will be left in place.
    pub spared_scheduler_jobs: u64,
}

/// Outcome of a maintenance run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceResult {
    /// `drain` | `clean` | `retry`.
    pub action: String,
    /// Jobs removed / retried.
    pub affected: u64,
    /// Jobs left in place on purpose (flows, scheduler runs) or, for a
    /// retry, ids whose job no longer existed.
    pub skipped: u64,
    /// Stopped before the end (the connection was closed, the app quit).
    pub cancelled: bool,
}

/// Finished states `clean` works on. (Jobs that have not started are
/// `drain`'s business; active jobs are nobody's.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanState {
    Completed,
    Failed,
}

/// A queue-level maintenance operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueMaintenance {
    /// Remove every job that has not started: waiting, paused, prioritized
    /// and, unless `include_delayed` is false, delayed.
    Drain { include_delayed: bool },
    /// Remove the `state` jobs that finished more than `older_than_ms` ago
    /// (`0` = all of them).
    Clean {
        state: CleanState,
        older_than_ms: u64,
    },
    /// Put every failed job back in line.
    RetryAllFailed,
}

impl QueueMaintenance {
    /// The name the safety gate, the audit trail and MCP know it by. An
    /// agent retries "all failed" through `bulk_retry_jobs`.
    pub fn tool(&self) -> &'static str {
        match self {
            Self::Drain { .. } => "drain_queue",
            Self::Clean { .. } => "clean_queue",
            Self::RetryAllFailed => "bulk_retry_jobs",
        }
    }

    /// Verb for progress events and summaries.
    pub fn verb(&self) -> &'static str {
        match self {
            Self::Drain { .. } => "drain",
            Self::Clean { .. } => "clean",
            Self::RetryAllFailed => "retry",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_map_to_the_tool_the_gate_knows() {
        assert_eq!(
            QueueMaintenance::Drain {
                include_delayed: true
            }
            .tool(),
            "drain_queue"
        );
        assert_eq!(
            QueueMaintenance::Clean {
                state: CleanState::Failed,
                older_than_ms: 0
            }
            .tool(),
            "clean_queue"
        );
        // An agent's "retry all failed" is `bulk_retry_jobs` + a selector.
        assert_eq!(QueueMaintenance::RetryAllFailed.tool(), "bulk_retry_jobs");
    }
}
