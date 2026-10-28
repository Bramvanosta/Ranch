//! Arguments and results of the job operations: listing, reading,
//! single and bulk actions, adding a job.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::queues::QueueId;
use crate::contract::queues::FailingJobType;
use crate::domain;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListJobsArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
    pub state: JobState,
    /// 0-based offset into the state's list/sorted-set.
    pub offset: u64,
    /// Page size. The frontend uses 50 (target ≤100ms per page).
    pub limit: u64,
    /// For sorted-set states (Completed / Failed / Delayed / Prioritized),
    /// `true` returns highest-score-first (typical "newest first" UX,
    /// implemented via `ZREVRANGE`). List states (Waiting / Active /
    /// Paused) ignore this flag.
    pub newest_first: bool,
    /// case-insensitive substring of the job **id or name**,
    /// matched across the whole state (bounded scan), not the loaded page.
    #[serde(default)]
    pub search: Option<String>,
    /// case-insensitive substring of the failure reason.
    #[serde(default)]
    pub reason: Option<String>,
    /// Custom sort. `None` keeps the state's natural order.
    #[serde(default)]
    pub sort: Option<JobSort>,
}

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListJobsOutput {
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

impl From<domain::jobs::JobPage> for ListJobsOutput {
    fn from(v: domain::jobs::JobPage) -> Self {
        Self {
            jobs: v.jobs.into_iter().map(Into::into).collect(),
            total: v.total,
            state: v.state.into(),
            has_more: v.has_more,
            scanned: v.scanned,
            scan_capped: v.scan_capped,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GetJobArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GetJobLogsArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
    pub job_id: String,
    /// 0-based offset into the logs list. Default 0.
    pub offset: u64,
    /// Page size. The frontend uses 200 by default; that covers
    /// the realistic case; pathological multi-thousand-line cases get
    /// pagination via Previous/Next.
    pub limit: u64,
}

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JobLogsOutput {
    pub lines: Vec<String>,
    pub total: u64,
    pub has_more: bool,
}

/// Shared input shape for all three single-job actions. The action
/// kind is encoded in the Tauri command name (`retry_job` /
/// `remove_job` / `promote_job`); the body itself is identical.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JobActionArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
    pub job_id: String,
}

/// Reply for all three single-job actions. `new_state` is the
/// resulting BullMQ state on success (`waiting` for retry/promote,
/// `None` for remove since the job no longer exists). `action` is
/// the verb a frontend can use in success toasts ("retry" /
/// "remove" / "promote") without re-deriving from the command name.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JobActionResult {
    pub job_id: String,
    pub new_state: Option<JobState>,
    pub action: String,
}

/// Args for `add_job`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddJobArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
    pub job: NewJob,
}

/// The job Ranch created. `state` is where BullMQ's rules put it: `waiting`,
/// `paused` (queue paused), `delayed` or `prioritized`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AddJobResult {
    pub job_id: String,
    pub state: JobState,
}

/// Input for a bulk action over a list of jobs within a
/// single queue; cross-queue scope is not supported.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BulkActionArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
    pub job_ids: Vec<String>,
}

/// per-job failure entry collected during a bulk run.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BulkFailure {
    pub job_id: String,
    pub reason: String,
}

/// Outcome of a bulk action. The frontend's
/// `BulkActionBar` renders the summary from these fields. `cancelled`
/// is true when the actor's cancellation token fired mid-loop
/// (frontend-abandons-promise path = stays false; only true if the
/// actor is shutdown).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BulkActionResult {
    pub action: String,
    pub total: u32,
    pub succeeded: u32,
    pub failed: Vec<BulkFailure>,
    pub cancelled: bool,
}

/// Typed progress event emitted by the actor every N
/// completed jobs (see `BULK_PROGRESS_CHUNK` for chunk size). The
/// frontend subscribes via `@tauri-apps/api/event::listen("bulk:progress")`
/// and drives the progress bar live.
///
/// `queue_id` tells concurrent per-queue progress streams apart during a
/// cross-queue bulk run.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BulkProgressEvent {
    pub action: String,
    pub queue_id: QueueId,
    pub total: u32,
    pub completed: u32,
    pub failed_so_far: u32,
}

/// Args for `list_recent_jobs`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListRecentJobsArgs {
    pub connection_id: String,
    /// One queue; `None` = every queue of the connection, merged.
    #[serde(default)]
    pub queue_id: Option<QueueId>,
    /// Jobs wanted, clamped to 1–50 (0 = 15).
    #[serde(default)]
    pub limit: u32,
}

/// Args for `list_failed_jobs`. All filters are optional and combine.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListFailedJobsArgs {
    pub connection_id: String,
    pub offset: u32,
    /// Page size, clamped to 1–250.
    pub limit: u32,
    /// Exact queue name.
    #[serde(default)]
    pub queue: Option<String>,
    /// Exact job name.
    #[serde(default)]
    pub name: Option<String>,
    /// Case-insensitive substring of the failure reason.
    #[serde(default)]
    pub reason: Option<String>,
    /// Exact cause signature (`FailureCause::signature`).
    #[serde(default)]
    pub signature: Option<String>,
}

/// The failed jobs of a queue, or of the whole connection,
/// grouped by cause.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListFailureCausesArgs {
    pub connection_id: String,
    /// Exact queue name; none = every queue of the connection.
    #[serde(default)]
    pub queue: Option<String>,
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
    #[serde(default)]
    pub processed_by: Option<String>,
    /// Time left on the job's lock (`…:lock`, ms) while a
    /// worker holds it; a worker renews it as long as the job runs. Only
    /// read for running jobs.
    #[serde(default)]
    pub lock_ttl_ms: Option<i64>,
}

impl From<domain::jobs::Job> for Job {
    fn from(v: domain::jobs::Job) -> Self {
        Self {
            job_ref: v.job_ref.into(),
            name: v.name,
            state: v.state.into(),
            data: v.data,
            opts: v.opts,
            attempts_made: v.attempts_made,
            timestamp: v.timestamp,
            processed_on: v.processed_on,
            finished_on: v.finished_on,
            delay: v.delay,
            failed_reason: v.failed_reason,
            stacktrace: v.stacktrace,
            return_value: v.return_value,
            parent_key: v.parent_key,
            progress: v.progress,
            processed_by: v.processed_by,
            lock_ttl_ms: v.lock_ttl_ms,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecentJobs {
    /// Running jobs first (latest started first), then finished ones, latest first.
    pub jobs: Vec<Job>,
    /// Queues whose sets could not be read, reported, never fatal.
    pub unreadable_queues: Vec<String>,
}

impl From<domain::jobs::RecentJobs> for RecentJobs {
    fn from(v: domain::jobs::RecentJobs) -> Self {
        Self {
            jobs: v.jobs.into_iter().map(Into::into).collect(),
            unreadable_queues: v.unreadable_queues,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FailedQueueCount {
    pub queue_id: QueueId,
    pub queue_name: String,
    /// `ZCARD` of the queue's failed set, exact.
    pub failed: u32,
}

impl From<domain::failures::FailedQueueCount> for FailedQueueCount {
    fn from(v: domain::failures::FailedQueueCount) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            queue_name: v.queue_name,
            failed: v.failed,
        }
    }
}

/// One page of the cross-queue failed view.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FailedJobsPage {
    /// Newest failure first. `state` is always `failed`.
    pub jobs: Vec<Job>,
    /// Jobs matching the filter among the scanned ones: what pagination runs on.
    pub total: u32,
    /// Every failed job on the connection (sum of the queues' counts), exact.
    pub total_failed: u32,
    /// Per-queue counts, most failing first; queues without failures omitted.
    pub queues: Vec<FailedQueueCount>,
    /// Most failing job types among the scanned jobs (before the name /
    /// reason filter, so the summary stays a way to *choose* a filter).
    pub types: Vec<FailingJobType>,
    /// How many of the newest failed jobs were looked at…
    pub scanned: u32,
    /// …and whether older ones exist beyond the scan.
    pub scan_capped: bool,
    /// Queues whose failed set could not be read (reported, not fatal).
    pub unreadable_queues: Vec<String>,
}

impl From<domain::failures::FailedJobsPage> for FailedJobsPage {
    fn from(v: domain::failures::FailedJobsPage) -> Self {
        Self {
            jobs: v.jobs.into_iter().map(Into::into).collect(),
            total: v.total,
            total_failed: v.total_failed,
            queues: v.queues.into_iter().map(Into::into).collect(),
            types: v.types.into_iter().map(Into::into).collect(),
            scanned: v.scanned,
            scan_capped: v.scan_capped,
            unreadable_queues: v.unreadable_queues,
        }
    }
}

/// One cause: the failures whose reasons only differ by the values in them.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FailureCause {
    /// The reason with its values replaced (`<n>`, `<str>`, `<url>`…).
    /// Empty = the jobs recorded no reason.
    pub signature: String,
    /// The newest reason of the cause, as written.
    pub sample: String,
    pub count: u32,
    pub first_at: i64,
    pub last_at: i64,
    /// Job types of the cause, most frequent first (at most five).
    pub job_names: Vec<String>,
    /// Queues of the cause, most frequent first.
    pub queue_names: Vec<String>,
    /// Failures per hour over the last 24 hours, oldest first.
    pub trend: Vec<u32>,
    /// Every scanned job of the cause, newest first: what "Retry N" and
    /// "Remove N" act on.
    pub jobs: Vec<JobRef>,
}

impl From<domain::failures::FailureCause> for FailureCause {
    fn from(v: domain::failures::FailureCause) -> Self {
        Self {
            signature: v.signature,
            sample: v.sample,
            count: v.count,
            first_at: v.first_at,
            last_at: v.last_at,
            job_names: v.job_names,
            queue_names: v.queue_names,
            trend: v.trend,
            jobs: v.jobs.into_iter().map(Into::into).collect(),
        }
    }
}

/// The failed jobs of a queue (or of a connection), grouped by cause.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FailureCauses {
    /// Most frequent first.
    pub causes: Vec<FailureCause>,
    /// Failed jobs looked at…
    pub scanned: u32,
    /// …and whether older ones exist beyond the scan.
    pub scan_capped: bool,
    /// Every failed job of the queue / connection, exact.
    pub total_failed: u32,
    /// Queues whose failed set could not be read (reported, not fatal).
    pub unreadable_queues: Vec<String>,
}

impl From<domain::failures::FailureCauses> for FailureCauses {
    fn from(v: domain::failures::FailureCauses) -> Self {
        Self {
            causes: v.causes.into_iter().map(Into::into).collect(),
            scanned: v.scanned,
            scan_capped: v.scan_capped,
            total_failed: v.total_failed,
            unreadable_queues: v.unreadable_queues,
        }
    }
}

/// One job of a flow. The tree is flat (`parent_key` links the nodes) so
/// it crosses the IPC boundary without recursive types and feeds a graph
/// layout directly.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowNode {
    /// `{prefix}:{queue}:{id}`, unique within the flow.
    pub key: String,
    pub job_ref: JobRef,
    /// `None` for the root.
    pub parent_key: Option<String>,
    pub name: String,
    pub state: JobState,
    pub timestamp: Option<i64>,
    /// `finishedOn − processedOn`, when both are known.
    pub duration_ms: Option<i64>,
    pub failed_reason: Option<String>,
    /// The job hash no longer exists (removed by `removeOnComplete`, a clean…):
    /// the parent still references it.
    pub missing: bool,
    /// What a finished job returned (raw JSON), if anything.
    #[serde(default)]
    pub return_value: Option<String>,
    /// This job failed for good and its parent waits for it
    /// forever (no `failParentOnFailure` / `ignoreDependencyOnFailure`…).
    #[serde(default)]
    pub blocks_parent: bool,
}

impl From<domain::flows::FlowNode> for FlowNode {
    fn from(v: domain::flows::FlowNode) -> Self {
        Self {
            key: v.key,
            job_ref: v.job_ref.into(),
            parent_key: v.parent_key,
            name: v.name,
            state: v.state.into(),
            timestamp: v.timestamp,
            duration_ms: v.duration_ms,
            failed_reason: v.failed_reason,
            missing: v.missing,
            return_value: v.return_value,
            blocks_parent: v.blocks_parent,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowTree {
    /// Root first, then breadth-first.
    pub nodes: Vec<FlowNode>,
    pub root_key: String,
    /// The job the flow was asked for.
    pub focus_key: String,
    /// The flow holds more than [`MAX_NODES`] jobs; the rest is not shown.
    pub truncated: bool,
    /// A parent waits on a child that failed for good.
    #[serde(default)]
    pub blocked: bool,
}

impl From<domain::flows::FlowTree> for FlowTree {
    fn from(v: domain::flows::FlowTree) -> Self {
        Self {
            nodes: v.nodes.into_iter().map(Into::into).collect(),
            root_key: v.root_key,
            focus_key: v.focus_key,
            truncated: v.truncated,
            blocked: v.blocked,
        }
    }
}

/// A flow root found in a queue (the queue's Flows tab).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowRoot {
    pub job_ref: JobRef,
    pub name: String,
    pub state: JobState,
    pub timestamp: Option<i64>,
    /// Direct children, by what became of them.
    pub children_total: u64,
    pub children_pending: u64,
    pub children_completed: u64,
    pub children_failed: u64,
    /// The root waits on a child that failed for good.
    #[serde(default)]
    pub blocked: bool,
}

impl From<domain::flows::FlowRoot> for FlowRoot {
    fn from(v: domain::flows::FlowRoot) -> Self {
        Self {
            job_ref: v.job_ref.into(),
            name: v.name,
            state: v.state.into(),
            timestamp: v.timestamp,
            children_total: v.children_total,
            children_pending: v.children_pending,
            children_completed: v.children_completed,
            children_failed: v.children_failed,
            blocked: v.blocked,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowRoots {
    /// Newest first.
    pub roots: Vec<FlowRoot>,
    /// Jobs looked at.
    pub scanned: u64,
    /// The queue holds more finished jobs than discovery looks at: older
    /// flows are not listed.
    pub capped: bool,
}

impl From<domain::flows::FlowRoots> for FlowRoots {
    fn from(v: domain::flows::FlowRoots) -> Self {
        Self {
            roots: v.roots.into_iter().map(Into::into).collect(),
            scanned: v.scanned,
            capped: v.capped,
        }
    }
}

impl FlowTree {
    /// The tree as nested JSON: what an agent reads (`get_flow`).
    pub fn to_nested_json(&self) -> serde_json::Value {
        let mut children: HashMap<&str, Vec<&FlowNode>> = HashMap::new();
        for node in &self.nodes {
            if let Some(parent) = node.parent_key.as_deref() {
                children.entry(parent).or_default().push(node);
            }
        }
        fn build(node: &FlowNode, children: &HashMap<&str, Vec<&FlowNode>>) -> serde_json::Value {
            let kids: Vec<serde_json::Value> = children
                .get(node.key.as_str())
                .map(|list| list.iter().map(|c| build(c, children)).collect())
                .unwrap_or_default();
            serde_json::json!({
                "queue": node.job_ref.queue_name,
                "queueId": node.job_ref.queue_id,
                "jobId": node.job_ref.id,
                "name": node.name,
                "state": node.state,
                "durationMs": node.duration_ms,
                "failedReason": node.failed_reason,
                "missing": node.missing,
                "children": kids,
            })
        }
        let root = self.nodes.iter().find(|n| n.key == self.root_key);
        serde_json::json!({
            "focus": self.focus_key,
            "truncated": self.truncated,
            "jobCount": self.nodes.len(),
            "root": root.map(|r| build(r, &children)),
        })
    }
}

/// What the user (or an agent) asks for.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NewJob {
    pub name: String,
    /// The payload, as JSON text (`{"to":"a@b.c"}`). Any JSON value.
    pub data: String,
    /// Milliseconds before the job may run. `0` = now.
    #[serde(default)]
    pub delay_ms: u64,
    /// Total attempts (BullMQ `attempts`). `None` = BullMQ's default (1).
    #[serde(default)]
    pub attempts: Option<u32>,
    /// `0` = none; otherwise 1 (highest) ‥ 2,097,152 (lowest).
    #[serde(default)]
    pub priority: u32,
    /// Process before the jobs already waiting.
    #[serde(default)]
    pub lifo: bool,
    /// How long BullMQ waits before the next attempt.
    #[serde(default)]
    pub backoff: Option<Backoff>,
    /// A custom job ID; `None` = BullMQ's counter.
    #[serde(default)]
    pub job_id: Option<String>,
    /// Remove the job once it completes (`removeOnComplete`).
    #[serde(default)]
    pub remove_on_complete: bool,
    /// Remove the job once it fails for good (`removeOnFail`).
    #[serde(default)]
    pub remove_on_fail: bool,
}

impl From<domain::jobs::NewJob> for NewJob {
    fn from(v: domain::jobs::NewJob) -> Self {
        Self {
            name: v.name,
            data: v.data,
            delay_ms: v.delay_ms,
            attempts: v.attempts,
            priority: v.priority,
            lifo: v.lifo,
            backoff: v.backoff.map(Into::into),
            job_id: v.job_id,
            remove_on_complete: v.remove_on_complete,
            remove_on_fail: v.remove_on_fail,
        }
    }
}

/// BullMQ's built-in backoff strategies (`opts.backoff`).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Backoff {
    pub kind: BackoffKind,
    pub delay_ms: u64,
}

impl From<domain::jobs::Backoff> for Backoff {
    fn from(v: domain::jobs::Backoff) -> Self {
        Self {
            kind: v.kind.into(),
            delay_ms: v.delay_ms,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BackoffKind {
    Fixed,
    Exponential,
}

impl From<domain::jobs::BackoffKind> for BackoffKind {
    fn from(v: domain::jobs::BackoffKind) -> Self {
        match v {
            domain::jobs::BackoffKind::Fixed => Self::Fixed,
            domain::jobs::BackoffKind::Exponential => Self::Exponential,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum JobSortField {
    /// Job name, then id.
    Name,
    /// `timestamp`: when the job was added.
    QueuedAt,
    /// `finishedOn − processedOn`; jobs without a duration go last.
    Duration,
}

impl From<domain::jobs::JobSortField> for JobSortField {
    fn from(v: domain::jobs::JobSortField) -> Self {
        match v {
            domain::jobs::JobSortField::Name => Self::Name,
            domain::jobs::JobSortField::QueuedAt => Self::QueuedAt,
            domain::jobs::JobSortField::Duration => Self::Duration,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobSort {
    pub field: JobSortField,
    pub descending: bool,
}

impl From<domain::jobs::JobSort> for JobSort {
    fn from(v: domain::jobs::JobSort) -> Self {
        Self {
            field: v.field.into(),
            descending: v.descending,
        }
    }
}

impl From<NewJob> for domain::jobs::NewJob {
    fn from(v: NewJob) -> Self {
        Self {
            name: v.name,
            data: v.data,
            delay_ms: v.delay_ms,
            attempts: v.attempts,
            priority: v.priority,
            lifo: v.lifo,
            backoff: v.backoff.map(Into::into),
            job_id: v.job_id,
            remove_on_complete: v.remove_on_complete,
            remove_on_fail: v.remove_on_fail,
        }
    }
}

impl From<Backoff> for domain::jobs::Backoff {
    fn from(v: Backoff) -> Self {
        Self {
            kind: v.kind.into(),
            delay_ms: v.delay_ms,
        }
    }
}

impl From<BackoffKind> for domain::jobs::BackoffKind {
    fn from(v: BackoffKind) -> Self {
        match v {
            BackoffKind::Fixed => Self::Fixed,
            BackoffKind::Exponential => Self::Exponential,
        }
    }
}

impl From<JobSortField> for domain::jobs::JobSortField {
    fn from(v: JobSortField) -> Self {
        match v {
            JobSortField::Name => Self::Name,
            JobSortField::QueuedAt => Self::QueuedAt,
            JobSortField::Duration => Self::Duration,
        }
    }
}

impl From<JobSort> for domain::jobs::JobSort {
    fn from(v: JobSort) -> Self {
        Self {
            field: v.field.into(),
            descending: v.descending,
        }
    }
}

/// A job state, by its code: the common lifecycle (`waiting`, `active`,
/// `completed`, `failed`, `delayed`, `paused`) and whatever states the
/// connection's backend adds (BullMQ: `prioritized`, `waitingChildren`,
/// `stalled`). The backend that receives one resolves it against the
/// states it declares.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct JobState(pub String);

impl JobState {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<domain::jobs::JobState> for JobState {
    fn from(v: domain::jobs::JobState) -> Self {
        Self(v.as_str().to_string())
    }
}

impl ListJobsArgs {
    /// The page the backend reads, once the operation has resolved the
    /// state against the states the backend declares.
    pub fn query(self, state: domain::jobs::JobState) -> domain::jobs::JobPageQuery {
        domain::jobs::JobPageQuery {
            queue_id: self.queue_id.into(),
            state,
            offset: self.offset,
            limit: self.limit,
            newest_first: self.newest_first,
            search: self.search,
            reason: self.reason,
            sort: self.sort.map(Into::into),
        }
    }
}

impl From<GetJobArgs> for domain::jobs::JobTarget {
    fn from(v: GetJobArgs) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            job_id: v.job_id,
        }
    }
}

impl From<JobActionArgs> for domain::jobs::JobTarget {
    fn from(v: JobActionArgs) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            job_id: v.job_id,
        }
    }
}

impl From<BulkActionArgs> for domain::jobs::JobBatch {
    fn from(v: BulkActionArgs) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            job_ids: v.job_ids,
        }
    }
}

impl From<GetJobLogsArgs> for domain::jobs::JobLogsQuery {
    fn from(v: GetJobLogsArgs) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            job_id: v.job_id,
            offset: v.offset,
            limit: v.limit,
        }
    }
}

impl From<ListRecentJobsArgs> for domain::jobs::RecentJobsQuery {
    fn from(v: ListRecentJobsArgs) -> Self {
        Self {
            queue_id: v.queue_id.map(Into::into),
            limit: v.limit,
        }
    }
}

impl From<ListFailedJobsArgs> for domain::failures::FailedJobsQuery {
    fn from(v: ListFailedJobsArgs) -> Self {
        Self {
            offset: v.offset,
            limit: v.limit,
            filter: domain::failures::FailedFilter {
                queue: v.queue,
                name: v.name,
                reason: v.reason,
                signature: v.signature,
            },
        }
    }
}

impl From<domain::jobs::JobLogs> for JobLogsOutput {
    fn from(v: domain::jobs::JobLogs) -> Self {
        Self {
            lines: v.lines,
            total: v.total,
            has_more: v.has_more,
        }
    }
}

impl From<domain::jobs::BulkActionResult> for BulkActionResult {
    fn from(v: domain::jobs::BulkActionResult) -> Self {
        Self {
            action: v.action,
            total: v.total,
            succeeded: v.succeeded,
            failed: v
                .failed
                .into_iter()
                .map(|f| BulkFailure {
                    job_id: f.job_id,
                    reason: f.reason,
                })
                .collect(),
            cancelled: v.cancelled,
        }
    }
}

impl From<domain::jobs::BulkProgress> for BulkProgressEvent {
    fn from(v: domain::jobs::BulkProgress) -> Self {
        Self {
            action: v.action,
            queue_id: v.queue_id.into(),
            total: v.total,
            completed: v.completed,
            failed_so_far: v.failed_so_far,
        }
    }
}

impl From<domain::jobs::JobActionResult> for JobActionResult {
    fn from(v: domain::jobs::JobActionResult) -> Self {
        Self {
            job_id: v.job_id,
            new_state: v.new_state.map(Into::into),
            action: v.action,
        }
    }
}

impl From<domain::jobs::AddJobResult> for AddJobResult {
    fn from(v: domain::jobs::AddJobResult) -> Self {
        Self {
            job_id: v.job_id,
            state: v.state.into(),
        }
    }
}

/// A job: its queue's id, the queue's name for display, and the job's id
/// within the queue.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct JobRef {
    pub queue_id: QueueId,
    pub queue_name: String,
    pub id: String,
}

impl From<domain::jobs::JobRef> for JobRef {
    fn from(v: domain::jobs::JobRef) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            queue_name: v.queue_name,
            id: v.id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::bullmq_redis::bullmq_schema::flow::parse_job_key;

    fn node(key: &str, parent: Option<&str>, state: &str) -> FlowNode {
        let job_ref = parse_job_key(key, "bull").unwrap();
        FlowNode {
            key: key.into(),
            name: format!("job-{}", job_ref.id),
            job_ref: job_ref.into(),
            parent_key: parent.map(Into::into),
            state: JobState(state.into()),
            timestamp: Some(1),
            duration_ms: None,
            failed_reason: None,
            missing: false,
            return_value: None,
            blocks_parent: false,
        }
    }

    #[test]
    fn nests_the_flat_tree_for_agents() {
        let tree = FlowTree {
            nodes: vec![
                node("bull:a:1", None, "waitingChildren"),
                node("bull:b:2", Some("bull:a:1"), "completed"),
                node("bull:b:3", Some("bull:a:1"), "failed"),
                node("bull:a:4", Some("bull:b:3"), "waiting"),
            ],
            root_key: "bull:a:1".into(),
            focus_key: "bull:b:3".into(),
            truncated: false,
            blocked: false,
        };
        let json = tree.to_nested_json();
        assert_eq!(json["jobCount"], 4);
        assert_eq!(json["focus"], "bull:b:3");
        assert_eq!(json["root"]["jobId"], "1");
        assert_eq!(json["root"]["state"], "waitingChildren");
        let kids = json["root"]["children"].as_array().unwrap();
        assert_eq!(kids.len(), 2);
        assert_eq!(kids[1]["queue"], "b");
        assert_eq!(kids[1]["children"][0]["jobId"], "4");
        assert_eq!(kids[0]["children"].as_array().unwrap().len(), 0);
    }
}
