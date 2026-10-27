//! Queue maintenance: **drain**, **clean**, **retry all
//! failed** — and every retry.
//!
//! Same rule as `add_job`: each operation is a
//! Ranch-owned Lua script, a port of the upstream one, run in **batches** so
//! a queue of 200,000 jobs never blocks Redis for long and the actor can
//! report progress and be cancelled between two batches.
//!
//! What Ranch deliberately does not touch, and reports as *skipped*:
//!
//!   - members of a **flow** (a child, or a parent still tracking children):
//!     removing one means rewriting the other side's bookkeeping;
//!   - the pending run of a **job scheduler**: draining it would silently
//!     stop the schedule (BullMQ's own `drain` spares it too);
//!   - **active** jobs: a worker is holding them.
//!
//! This module is the pure half: the operations, their script sources, and
//! how a script reply is read.

use serde::{Deserialize, Serialize};
use specta::Type;

const PRELUDE: &str = include_str!("lua/_prelude.lua");

/// Jobs per script call. Small enough to keep Redis responsive, large
/// enough that 100,000 jobs are ~200 round trips.
pub const BATCH: usize = 500;

pub(super) fn with_prelude(body: &str) -> String {
    format!("{PRELUDE}\n{body}")
}

pub fn retry_jobs_lua() -> String {
    with_prelude(include_str!("lua/retry_jobs.lua"))
}

pub fn promote_jobs_lua() -> String {
    with_prelude(include_str!("lua/promote_jobs.lua"))
}

pub fn retry_all_failed_lua() -> String {
    with_prelude(include_str!("lua/retry_all_failed.lua"))
}

pub fn drain_queue_lua() -> String {
    with_prelude(include_str!("lua/drain_queue.lua"))
}

pub fn clean_jobs_lua() -> String {
    with_prelude(include_str!("lua/clean_jobs.lua"))
}

/// Finished states `clean` works on. (Jobs that have not started are
/// `drain`'s business; active jobs are nobody's.)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CleanState {
    Completed,
    Failed,
}

/// A queue-level maintenance operation.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QueueMaintenance {
    /// Remove every job that has not started: waiting, paused, prioritized
    /// and — unless `include_delayed` is false — delayed.
    #[serde(rename_all = "camelCase")]
    Drain { include_delayed: bool },
    /// Remove the `state` jobs that finished more than `older_than_ms` ago
    /// (`0` = all of them).
    #[serde(rename_all = "camelCase")]
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

/// What a maintenance operation would touch — shown before confirming.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MaintenancePreview {
    /// Jobs the operation will act on. For drain this is an upper bound:
    /// flow members are only recognised job by job, while running.
    pub affected: u64,
    /// Drain only — pending scheduler runs that will be left in place.
    pub spared_scheduler_jobs: u64,
}

/// Outcome of a maintenance run.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceResult {
    /// `drain` | `clean` | `retry`.
    pub action: String,
    /// Jobs removed / retried.
    pub affected: u64,
    /// Jobs left in place on purpose (flows, scheduler runs) — or, for a
    /// retry, ids whose job no longer existed.
    pub skipped: u64,
    /// Stopped before the end (the connection was closed, the app quit).
    pub cancelled: bool,
}

/// `{ a, b, c }` as a script returns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatchReply {
    pub first: u64,
    pub second: u64,
    pub third: u64,
}

impl From<(i64, i64, i64)> for BatchReply {
    fn from((a, b, c): (i64, i64, i64)) -> Self {
        let n = |v: i64| u64::try_from(v).unwrap_or(0);
        Self {
            first: n(a),
            second: n(b),
            third: n(c),
        }
    }
}

/// Result of one id in `retry_jobs.lua` / `promote_jobs.lua`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveCode {
    /// Retried / promoted.
    Moved,
    JobNotFound,
    /// Not in the state the operation starts from (failed / delayed).
    WrongState,
}

impl MoveCode {
    pub fn from_script(code: i64) -> Self {
        match code {
            1 => Self::Moved,
            -3 => Self::WrongState,
            _ => Self::JobNotFound,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_script_gets_the_prelude_and_stays_msgpack_free() {
        for script in [
            promote_jobs_lua(),
            retry_jobs_lua(),
            retry_all_failed_lua(),
            drain_queue_lua(),
            clean_jobs_lua(),
        ] {
            assert!(script.starts_with("--[["), "prelude first");
            assert!(script.contains("local function retryFailedJob"));
            assert!(script.contains("local function isFlowMember"));
            assert!(!script.contains("cmsgpack"));
            // One `rcall` definition: the prelude's.
            assert_eq!(script.matches("local rcall = redis.call").count(), 1);
        }
    }

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

    #[test]
    fn wire_format_is_a_tagged_camel_case_union() {
        let json = serde_json::to_value(QueueMaintenance::Clean {
            state: CleanState::Completed,
            older_than_ms: 86_400_000,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "clean", "state": "completed", "olderThanMs": 86400000})
        );
        let drain: QueueMaintenance =
            serde_json::from_value(serde_json::json!({"kind": "drain", "includeDelayed": false}))
                .unwrap();
        assert_eq!(
            drain,
            QueueMaintenance::Drain {
                include_delayed: false
            }
        );
        let retry: QueueMaintenance =
            serde_json::from_value(serde_json::json!({"kind": "retryAllFailed"})).unwrap();
        assert_eq!(retry, QueueMaintenance::RetryAllFailed);
    }

    #[test]
    fn script_replies_are_read_defensively() {
        assert_eq!(
            BatchReply::from((3, -1, 7)),
            BatchReply {
                first: 3,
                second: 0,
                third: 7
            }
        );
        assert_eq!(MoveCode::from_script(1), MoveCode::Moved);
        assert_eq!(MoveCode::from_script(-3), MoveCode::WrongState);
        assert_eq!(MoveCode::from_script(-1), MoveCode::JobNotFound);
        assert_eq!(MoveCode::from_script(42), MoveCode::JobNotFound);
    }
}
