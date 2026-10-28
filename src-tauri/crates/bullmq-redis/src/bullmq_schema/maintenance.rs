//! Queue maintenance: **drain**, **clean**, **retry all
//! failed**, and every retry.
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
