//! A job, where it is, and the lists of jobs the backends answer with.

use crate::queues::QueueId;

/// A job: its queue and its id within it (a string; BullMQ counts them up
/// as numbers encoded as strings). The queue's name comes along for
/// display.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JobRef {
    pub queue_id: QueueId,
    pub queue_name: String,
    pub id: String,
}

/// A single decoded BullMQ job. All fields except `data` / `opts` /
/// `return_value` / `parent_key` are direct field translations from the
/// Redis hash; the JSON-encoded ones are parsed once at decode time so
/// feature code receives structured `serde_json::Value`.
///
/// `state` is the heuristic state inferred from job-hash fields alone.
/// Callers performing a list-membership query can override it.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub job_ref: JobRef,
    pub name: String,
    pub state: JobState,
    /// Job payload: the raw JSON-encoded string BullMQ stored in the
    /// `data` field. The decoder VALIDATES that this is parseable JSON
    /// (returns `SchemaError::InvalidJson` if not), but stores the
    /// original string so the wire shape stays simple. The frontend
    /// parses on demand: `JSON.parse(job.data)`.
    pub data: String,
    /// Job options: same treatment as `data`. Validated as JSON at
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
    /// Job return value: raw JSON-encoded string when present (BullMQ
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
    pub processed_by: Option<String>,
    /// Time left on the job's lock (`…:lock`, ms) while a
    /// worker holds it; a worker renews it as long as the job runs. Only
    /// read for running jobs.
    pub lock_ttl_ms: Option<i64>,
}

/// One page of a state's jobs.
#[derive(Debug, Clone, PartialEq)]
pub struct JobPage {
    pub jobs: Vec<Job>,
    /// Jobs in the state or, with a search / reason filter, jobs matching it.
    pub total: u64,
    pub state: JobState,
    pub has_more: bool,
    /// Jobs a search / sort looked at; `None` on the plain
    /// range-read path (nothing is scanned there).
    pub scanned: Option<u64>,
    /// The state holds more jobs than a query scans
    /// (`job_query::SCAN_CAP`): older jobs were not considered.
    pub scan_capped: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecentJobs {
    /// Running jobs first (latest started first), then finished ones, latest first.
    pub jobs: Vec<Job>,
    /// Queues whose sets could not be read, reported, never fatal.
    pub unreadable_queues: Vec<String>,
}

/// What the user (or an agent) asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    pub name: String,
    /// The payload, as JSON text (`{"to":"a@b.c"}`). Any JSON value.
    pub data: String,
    /// Milliseconds before the job may run. `0` = now.
    pub delay_ms: u64,
    /// Total attempts (BullMQ `attempts`). `None` = BullMQ's default (1).
    pub attempts: Option<u32>,
    /// `0` = none; otherwise 1 (highest) ‥ 2,097,152 (lowest).
    pub priority: u32,
    /// Process before the jobs already waiting.
    pub lifo: bool,
    /// How long BullMQ waits before the next attempt.
    pub backoff: Option<Backoff>,
    /// A custom job ID; `None` = BullMQ's counter.
    pub job_id: Option<String>,
    /// Remove the job once it completes (`removeOnComplete`).
    pub remove_on_complete: bool,
    /// Remove the job once it fails for good (`removeOnFail`).
    pub remove_on_fail: bool,
}

/// BullMQ's built-in backoff strategies (`opts.backoff`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backoff {
    pub kind: BackoffKind,
    pub delay_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackoffKind {
    Fixed,
    Exponential,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobSortField {
    /// Job name, then id.
    Name,
    /// `timestamp`: when the job was added.
    QueuedAt,
    /// `finishedOn − processedOn`; jobs without a duration go last.
    Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobSort {
    pub field: JobSortField,
    pub descending: bool,
}

/// Where a job is. A job is *in* one state at a time. The common
/// lifecycle has a variant each; a state one queue system adds is `Other`,
/// described by the backend that keeps it, so the domain does not grow a
/// variant per system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobState {
    Waiting,
    Active,
    Completed,
    Failed,
    Delayed,
    /// Jobs held while their queue is paused. Whether the queue itself
    /// is paused is on `Queue.is_paused`.
    Paused,
    Other(BackendState),
}

/// A job state one backend adds to the common lifecycle, as that backend
/// describes it. Backends declare theirs as constants, so a state stays a
/// small `Copy` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BackendState {
    /// The state as arguments and results name it (`prioritized`).
    pub code: &'static str,
    /// The state as a screen names it.
    pub label: &'static str,
    pub tone: Tone,
}

/// Which look a state borrows from the common lifecycle's: pending as
/// waiting, running as active, success as completed, warning as delayed,
/// danger as failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tone {
    Neutral,
    Pending,
    Running,
    Success,
    Warning,
    Danger,
}

impl JobState {
    /// The lifecycle every backend keeps.
    pub const COMMON: [JobState; 6] = [
        JobState::Waiting,
        JobState::Active,
        JobState::Completed,
        JobState::Failed,
        JobState::Delayed,
        JobState::Paused,
    ];

    /// The state's code (`waiting`, `prioritized`…): what crosses the wire
    /// and what an argument names.
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Waiting => "waiting",
            JobState::Active => "active",
            JobState::Completed => "completed",
            JobState::Failed => "failed",
            JobState::Delayed => "delayed",
            JobState::Paused => "paused",
            JobState::Other(state) => state.code,
        }
    }

    /// The state as a screen names it.
    pub fn label(self) -> &'static str {
        match self {
            JobState::Waiting => "Waiting",
            JobState::Active => "Active",
            JobState::Completed => "Completed",
            JobState::Failed => "Failed",
            JobState::Delayed => "Delayed",
            JobState::Paused => "Paused",
            JobState::Other(state) => state.label,
        }
    }

    /// The look the state borrows. Jobs held by a pause wait as delayed
    /// ones do.
    pub fn tone(self) -> Tone {
        match self {
            JobState::Waiting => Tone::Pending,
            JobState::Active => Tone::Running,
            JobState::Completed => Tone::Success,
            JobState::Failed => Tone::Danger,
            JobState::Delayed | JobState::Paused => Tone::Warning,
            JobState::Other(state) => state.tone,
        }
    }

    /// One of the lifecycle every backend keeps, rather than a state of
    /// one backend's own.
    pub fn is_common(self) -> bool {
        !matches!(self, JobState::Other(_))
    }

    /// The state among `known` whose code is `code`.
    pub fn from_code(code: &str, known: &[JobState]) -> Option<JobState> {
        known.iter().copied().find(|s| s.as_str() == code)
    }
}

impl std::fmt::Display for JobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Reply for all three single-job actions. `new_state` is the
/// resulting BullMQ state on success (`waiting` for retry/promote,
/// `None` for remove since the job no longer exists). `action` is
/// the verb a frontend can use in success toasts ("retry" /
/// "remove" / "promote") without re-deriving from the command name.
#[derive(Debug, Clone, PartialEq)]
pub struct JobActionResult {
    pub job_id: String,
    pub new_state: Option<JobState>,
    pub action: String,
}

/// The job Ranch created. `state` is where BullMQ's rules put it: `waiting`,
/// `paused` (queue paused), `delayed` or `prioritized`.
#[derive(Debug, Clone, PartialEq)]
pub struct AddJobResult {
    pub job_id: String,
    pub state: JobState,
}

/// One job of a queue, as an operation names it: the queue's id and the
/// job's id within it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobTarget {
    pub queue_id: QueueId,
    pub job_id: String,
}

/// Jobs of one queue a bulk action works through, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobBatch {
    pub queue_id: QueueId,
    pub job_ids: Vec<String>,
}

/// One page of a state's jobs: where it starts, how long it is, in what
/// order, and what the jobs must match.
#[derive(Debug, Clone, PartialEq)]
pub struct JobPageQuery {
    pub queue_id: QueueId,
    /// One of the states the backend declares.
    pub state: JobState,
    /// 0-based offset into the state's jobs.
    pub offset: u64,
    pub limit: u64,
    /// Newest first, for the states kept in time order; a state kept as a
    /// list keeps its own order.
    pub newest_first: bool,
    /// Case-insensitive substring of the job's id or name, matched across
    /// the whole state (bounded scan), not the loaded page.
    pub search: Option<String>,
    /// Case-insensitive substring of the failure reason.
    pub reason: Option<String>,
    /// `None` keeps the state's natural order.
    pub sort: Option<JobSort>,
}

/// A page of one job's log lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobLogsQuery {
    pub queue_id: QueueId,
    pub job_id: String,
    /// 0-based offset into the lines.
    pub offset: u64,
    pub limit: u64,
}

/// The log lines a job wrote, one page of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobLogs {
    pub lines: Vec<String>,
    pub total: u64,
    pub has_more: bool,
}

/// What ran last on one queue, or on every queue of the connection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecentJobsQuery {
    /// `None` = every queue, merged.
    pub queue_id: Option<QueueId>,
    /// Jobs wanted, clamped to 1–50 (0 = 15).
    pub limit: u32,
}

/// A job a bulk action could not act on, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkFailure {
    pub job_id: String,
    pub reason: String,
}

/// What a bulk action did: how many jobs it acted on, which it could
/// not, and whether it stopped before the end (the connection closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkActionResult {
    /// `retry` | `remove` | `promote`.
    pub action: String,
    pub total: u32,
    pub succeeded: u32,
    pub failed: Vec<BulkFailure>,
    pub cancelled: bool,
}

/// How far a bulk action or a maintenance run has got on one queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkProgress {
    pub action: String,
    pub queue_id: QueueId,
    pub total: u32,
    pub completed: u32,
    pub failed_so_far: u32,
}

/// Payload cap. Redis would take 512 MB; a hand-typed job never needs it.
pub const MAX_DATA_BYTES: usize = 1024 * 1024;

/// Drop the whitespace between tokens of an already-validated JSON text.
/// Parsing and re-serializing would do it too, but reorders the keys and
/// respells the numbers (`1.0` → `1.0`/`1`, `1e3` → `1000.0`): the payload a
/// worker receives should be the one that was typed.
pub fn minify_json(valid_json: &str) -> String {
    let mut out = String::with_capacity(valid_json.len());
    let mut in_string = false;
    let mut escaped = false;
    for c in valid_json.chars() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if !matches!(c, ' ' | '\t' | '\n' | '\r') {
            out.push(c);
        }
    }
    out
}
