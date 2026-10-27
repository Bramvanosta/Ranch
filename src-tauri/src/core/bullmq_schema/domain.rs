//! BullMQ schema domain types.
//!
//! Boundary types that the queue and job features (`commands/queues.rs`, `commands/jobs.rs`,
//! `core/connection_actor.rs`) consume. They derive `Serialize`,
//! `Deserialize`, `specta::Type` so they can cross the Tauri/MCP boundary
//! and appear in `bindings.ts` automatically.
//!
//! All field names are snake_case in Rust and camelCase on the wire (via
//! `#[serde(rename_all = "camelCase")]`).

use serde::{Deserialize, Serialize};
use specta::Type;

/// Identifier for a BullMQ queue. The `prefix` defaults to `"bull"` but is
/// configurable per BullMQ instance — we MUST persist whatever the user's
/// BullMQ deployment uses, otherwise the keys won't match.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct QueueRef {
    pub prefix: String,
    pub name: String,
}

/// Identifier for a single BullMQ job: queue + job id (string, BullMQ
/// generates them as monotonic numeric ids encoded as strings).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct JobRef {
    pub queue: QueueRef,
    pub id: String,
}

/// BullMQ job state. A job is *in* one state at a time; the state is
/// determined by which Redis list/set the job id appears in. The decoder
/// can also infer it from the job hash fields alone (a heuristic
/// — see `v5::infer_state`); the caller is free to override after a
/// membership query.
///
/// `Unknown` is the schema-not-recognized fallback. Per-version
/// decoders return it when the field encoding can't be classified.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Waiting,
    Active,
    Completed,
    Failed,
    Delayed,
    /// BullMQ v5+ — the prioritized sorted-set holds jobs that should jump
    /// the wait list based on `opts.priority`.
    Prioritized,
    /// Flow jobs whose parent has not yet completed.
    WaitingChildren,
    /// BullMQ v6+ canonical stalled set (v5 also has this list/set; v6
    /// promoted it to first-class for stalled-job recovery).
    Stalled,
    /// BullMQ stores paused jobs in a list when the queue is paused —
    /// surfaced here so `list_jobs` can read them. Queue-pause status
    /// itself is on `Queue.is_paused`.
    Paused,
    /// Schema-not-recognized fallback. The decoder returned this because
    /// the field encoding didn't match any known variant.
    Unknown,
}

/// Classifies the Redis structural type backing each
/// [`JobState`]. Drives whether `list_jobs` uses `LRANGE` (lists) or
/// `ZRANGE`/`ZREVRANGE` (sorted sets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateRedisShape {
    /// LRANGE iteration. BullMQ v6+ lists carry a `MARKER_TOKEN` sentinel
    /// at the head (see [`crate::core::bullmq_schema::v6::MARKER_TOKEN`])
    /// that callers MUST filter out before HGETALL.
    List,
    /// ZRANGE / ZREVRANGE iteration. Score is the relevant timestamp
    /// (e.g. `failedOn` for `Failed`, scheduled timestamp for `Delayed`).
    SortedSet,
    /// Schema-not-recognized fallback or a state we don't know how to
    /// fetch — `list_jobs` returns an empty page.
    Unsupported,
}

impl JobState {
    /// Map this state to its Redis structural shape. See [`StateRedisShape`].
    pub fn redis_shape(self) -> StateRedisShape {
        match self {
            JobState::Waiting | JobState::Active | JobState::Paused => StateRedisShape::List,
            JobState::Completed
            | JobState::Failed
            | JobState::Delayed
            | JobState::Prioritized
            | JobState::WaitingChildren
            | JobState::Stalled => StateRedisShape::SortedSet,
            JobState::Unknown => StateRedisShape::Unsupported,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_states_are_classified_correctly() {
        assert_eq!(JobState::Waiting.redis_shape(), StateRedisShape::List);
        assert_eq!(JobState::Active.redis_shape(), StateRedisShape::List);
        assert_eq!(JobState::Paused.redis_shape(), StateRedisShape::List);
    }

    #[test]
    fn sorted_set_states_are_classified_correctly() {
        for s in [
            JobState::Completed,
            JobState::Failed,
            JobState::Delayed,
            JobState::Prioritized,
            JobState::WaitingChildren,
            JobState::Stalled,
        ] {
            assert_eq!(s.redis_shape(), StateRedisShape::SortedSet, "state: {s:?}");
        }
    }

    #[test]
    fn unknown_state_is_unsupported() {
        assert_eq!(
            JobState::Unknown.redis_shape(),
            StateRedisShape::Unsupported
        );
    }

    // CronJob round-trips through the camelCase wire shape.
    #[test]
    fn cron_job_serializes_to_camel_case_wire_shape() {
        let cj = CronJob {
            key: "daily-report".into(),
            name: "send-daily-report".into(),
            pattern: Some("0 9 * * *".into()),
            every: None,
            tz: Some("Europe/Paris".into()),
            offset: Some(0),
            iteration_count: 5,
            next_run: 1_778_569_200_000,
            end_date: None,
            template: Some(r#"{"reportType":"summary"}"#.into()),
            paused: false,
        };
        let v = serde_json::to_value(&cj).unwrap();
        assert_eq!(v["key"], "daily-report");
        assert_eq!(v["iterationCount"], 5);
        assert_eq!(v["nextRun"], 1_778_569_200_000_i64);
        assert!(v.get("iteration_count").is_none(), "snake_case leaked");
        // Round-trip back through Deserialize.
        let back: CronJob = serde_json::from_value(v).unwrap();
        assert_eq!(back, cj);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct QueueCounters {
    pub waiting: u64,
    pub active: u64,
    pub completed: u64,
    pub failed: u64,
    pub delayed: u64,
    /// Jobs added with a `priority`: their own sorted set,
    /// processed once the wait list is empty.
    pub prioritized: u64,
    /// Jobs parked in the `paused` list: what was waiting when
    /// the queue was paused, plus what was added since.
    pub paused: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    pub queue_ref: QueueRef,
    pub counters: QueueCounters,
    pub is_paused: bool,
}

/// A single decoded BullMQ job. All fields except `data` / `opts` /
/// `return_value` / `parent_key` are direct field translations from the
/// Redis hash; the JSON-encoded ones are parsed once at decode time so
/// feature code receives structured `serde_json::Value`.
///
/// `state` is the heuristic state inferred from job-hash fields alone.
/// Callers performing a list-membership query can override it.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub job_ref: JobRef,
    pub name: String,
    pub state: JobState,
    /// Job payload — the raw JSON-encoded string BullMQ stored in the
    /// `data` field. The decoder VALIDATES that this is parseable JSON
    /// (returns `SchemaError::InvalidJson` if not), but stores the
    /// original string so the wire shape stays simple. The frontend
    /// parses on demand: `JSON.parse(job.data)`.
    pub data: String,
    /// Job options — same treatment as `data`. Validated as JSON at
    /// decode time, transmitted as the raw string.
    pub opts: String,
    pub attempts_made: u32,
    /// Milliseconds since epoch (BullMQ convention).
    pub timestamp: i64,
    pub processed_on: Option<i64>,
    pub finished_on: Option<i64>,
    pub delay: Option<i64>,
    pub failed_reason: Option<String>,
    /// Stacktrace lines as parsed from the `stacktrace` field (BullMQ
    /// JSON-encodes a `String[]`). Empty vec when no stacktrace stored.
    pub stacktrace: Vec<String>,
    /// Job return value — raw JSON-encoded string when present (BullMQ
    /// stores this for completed jobs in the `returnvalue` field).
    /// `None` when the field is absent or unparseable JSON. Validated at
    /// decode time but transmitted as the raw string.
    pub return_value: Option<String>,
    /// Parent flow-job key, when this job is a child in a BullMQ flow.
    pub parent_key: Option<String>,
    /// What the worker reported with `job.updateProgress()`: the
    /// raw JSON text, a number (`42`) or an object (`{"step":"upload"}`).
    /// `None` when absent or still at BullMQ's initial `0`.
    pub progress: Option<String>,
    /// The worker that took the job (`pb`, BullMQ ≥ 5), when
    /// it says.
    #[serde(default)]
    pub processed_by: Option<String>,
    /// Time left on the job's lock (`…:lock`, ms) while a
    /// worker holds it; a worker renews it as long as the job runs. Only
    /// read for running jobs.
    #[serde(default)]
    pub lock_ttl_ms: Option<i64>,
}

/// BullMQ repeatable / scheduler job. One row in the
/// `bull:{queue}:repeat` ZSET, decoded from the per-scheduler hash.
///
/// Exactly one of `pattern` / `every` is set for a valid scheduler:
/// `pattern` carries the cron expression (e.g. `0 9 * * *`); `every`
/// carries the fixed-interval millisecond delay. The
/// `handle_list_cron_jobs` skips (with `tracing::warn!`) any hash that
/// has neither or both.
///
/// `next_run` comes from the ZSET score (ms since epoch). `iteration_count`
/// comes from the `ic` hash field (the count of past iterations).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CronJob {
    /// Scheduler key — BullMQ's stable identifier (the developer-supplied
    /// id passed to `upsertJobScheduler`).
    pub key: String,
    /// The job name that BullMQ enqueues on each iteration.
    pub name: String,
    /// Cron expression. Mutually exclusive with `every`.
    pub pattern: Option<String>,
    /// Fixed interval in milliseconds. Mutually exclusive with `pattern`.
    pub every: Option<u64>,
    /// Optional IANA timezone string (e.g. `Europe/Paris`).
    pub tz: Option<String>,
    /// BullMQ-internal scheduling offset in ms. Surfaced for debugging;
    /// the UI hides it from the default columns.
    pub offset: Option<u64>,
    /// Iteration count — number of times this scheduler has already
    /// fired. From the `ic` hash field.
    pub iteration_count: u64,
    /// Next scheduled run timestamp (ms since epoch). From the ZSET score.
    pub next_run: i64,
    /// Optional end-of-schedule timestamp (ms since epoch). After this,
    /// BullMQ stops re-arming the scheduler.
    pub end_date: Option<i64>,
    /// Raw JSON template attached to enqueued jobs (BullMQ stores it as
    /// a JSON string in the `data` hash field). Validated as parseable
    /// JSON at decode time; transmitted as the raw string so the
    /// frontend can `JSON.parse(template)` on demand. None when the
    /// scheduler has no template (the common case).
    pub template: Option<String>,
    /// Pause flag. `true` when the scheduler has been
    /// paused via `pause_scheduler` (its entry is removed from the
    /// `:repeat` ZSET and the per-scheduler hash carries
    /// `paused = "1"`). The frontend's CronJobsView shows a "Paused"
    /// badge + a "Resume" button on these rows. `false` is the
    /// default for running schedulers.
    pub paused: bool,
}
