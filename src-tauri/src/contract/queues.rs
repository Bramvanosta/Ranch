//! Arguments and results of the queue operations: listing, workers,
//! schedulers, maintenance, metrics and events.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::jobs::JobRef;

use crate::domain;

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListQueuesArgs {
    pub connection_id: String,
}

/// How many jobs a queue holds in each state, by state code (`waiting`,
/// `prioritized`…). A state the backend did not count is absent: read it as
/// zero.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq, Default)]
#[serde(transparent)]
pub struct StateCounts(pub BTreeMap<String, u64>);

impl StateCounts {
    /// The jobs in the state `code` names, zero when it was not counted.
    pub fn get(&self, code: &str) -> u64 {
        self.0.get(code).copied().unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    pub id: QueueId,
    /// What the queue is called, for display.
    pub name: String,
    pub counts: StateCounts,
    pub is_paused: bool,
}

impl From<domain::queues::StateCounts> for StateCounts {
    fn from(c: domain::queues::StateCounts) -> Self {
        Self(c.iter().map(|(code, n)| (code.to_string(), n)).collect())
    }
}

impl From<domain::queues::Queue> for Queue {
    fn from(q: domain::queues::Queue) -> Self {
        Self {
            id: q.id.into(),
            name: q.name,
            counts: q.counts.into(),
            is_paused: q.is_paused,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListCronJobsArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
}

/// Args for `list_workers`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListWorkersArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
}

/// Args for `count_queue_workers`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CountQueueWorkersArgs {
    pub connection_id: String,
    pub queue_ids: Vec<QueueId>,
}

/// Args for `upsert_scheduler`. Saving an existing `scheduler.id` edits it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpsertSchedulerArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
    pub scheduler: SchedulerSpec,
}

/// The next runs of a schedule: what the sheet shows while typing.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRuns {
    /// Milliseconds since the epoch, soonest first.
    pub next_runs_ms: Vec<i64>,
}

/// The runs of a schedule in a window (the Schedulers tab's
/// "Next 24 hours"), at most `max` (≤ 500).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRunsWithinArgs {
    pub schedule: Schedule,
    /// Milliseconds after now: 86 400 000 for the next 24 hours.
    pub horizon_ms: f64,
    /// For `every`: the scheduler's next run, so the runs keep its grid.
    #[serde(default)]
    pub first_ms: Option<f64>,
    #[serde(default)]
    pub max: Option<u32>,
}

/// A maintenance operation on one queue, for the preview and for the run.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueMaintenanceArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
    pub operation: QueueMaintenance,
}

/// Args for `drain_queue`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DrainQueueArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
    /// Also remove delayed jobs (default: yes).
    pub include_delayed: bool,
}

/// Args for `clean_queue`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanQueueArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
    pub state: CleanState,
    /// Only jobs that finished more than this long ago. `0` = all.
    pub older_than_ms: u64,
}

/// Args for `pause_queue` / `resume_queue`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueActionArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
}

/// Result of `pause_queue` / `resume_queue`. `is_paused` is the
/// post-call state. `already_in_state` is true when the call was a
/// successful no-op (the queue was already in the target state),
/// matches the idempotency contract: pause-when-paused returns
/// `Ok({ is_paused: true, already_in_state: true })`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueActionResult {
    pub is_paused: bool,
    pub already_in_state: bool,
}

/// Args for `pause_scheduler` / `resume_scheduler`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerActionArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
    pub scheduler_key: String,
}

/// Result of `pause_scheduler` / `resume_scheduler`. Same shape as
/// `QueueActionResult` but with `paused` rather than `is_paused` so
/// the field name matches the `CronJob.paused` UI binding.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerActionResult {
    pub paused: bool,
    pub already_in_state: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GetQueueMetricsArgs {
    pub connection_id: String,
    pub queue_id: QueueId,
    /// The window (5m … 7d). Defaults to 24h.
    #[serde(default)]
    pub range: MetricsRange,
    /// "now" (ms), so that every queue of a connection-level call
    /// is bucketed against the same instant. `None` = the actor's clock.
    #[serde(default)]
    pub now_ms: Option<f64>,
}

/// Completed / failed over time for every queue of a
/// connection, from native metrics. Cheap: two reads per queue, one
/// pipeline, no job sampling.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GetThroughputArgs {
    pub connection_id: String,
    /// 1h, 6h or 24h are what the screens ask for; any window works.
    #[serde(default)]
    pub range: MetricsRange,
}

/// The Timeline's dragged window: done, failed, wait p95
/// against the rest of the visible range, failures by cause.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SummarizeWindowArgs {
    pub connection_id: String,
    /// The window, ms since epoch.
    pub from_ms: i64,
    pub to_ms: i64,
    /// Start of the visible range: "usual" is the range outside the window.
    pub range_from_ms: i64,
}

/// Follow (or stop following) a connection's BullMQ event
/// streams. Each window that follows counts; the reading stops when the
/// last one stops.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FollowConnectionEventsArgs {
    pub connection_id: String,
    pub follow: bool,
}

/// The `queue:events` event: what BullMQ wrote to the queues'
/// event streams since the last one, per queue.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueEventsNotice {
    pub connection_id: String,
    pub queues: Vec<QueueEventsSeen>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueEventsSeen {
    pub queue_id: QueueId,
    /// Event names seen (`added`, `completed`, `failed`, …), each once.
    pub events: Vec<String>,
    /// Entries read, capped by the read's COUNT.
    pub count: u32,
}

impl From<domain::events::QueueEventsNotice> for QueueEventsNotice {
    fn from(v: domain::events::QueueEventsNotice) -> Self {
        Self {
            connection_id: v.connection_id,
            queues: v
                .queues
                .into_iter()
                .map(|q| QueueEventsSeen {
                    queue_id: q.queue_id.into(),
                    events: q.events,
                    count: q.count,
                })
                .collect(),
        }
    }
}

impl From<ReadQueueEventsArgs> for domain::events::EventsQuery {
    fn from(v: ReadQueueEventsArgs) -> Self {
        Self {
            cursors: v.cursors.into_iter().map(Into::into).collect(),
            limit: v.limit,
        }
    }
}

impl From<SchedulerActionArgs> for domain::schedule::SchedulerTarget {
    fn from(v: SchedulerActionArgs) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            scheduler_key: v.scheduler_key,
        }
    }
}

impl From<domain::schedule::SchedulerActionResult> for SchedulerActionResult {
    fn from(v: domain::schedule::SchedulerActionResult) -> Self {
        Self {
            paused: v.paused,
            already_in_state: v.already_in_state,
        }
    }
}

impl From<domain::queues::QueueActionResult> for QueueActionResult {
    fn from(v: domain::queues::QueueActionResult) -> Self {
        Self {
            is_paused: v.is_paused,
            already_in_state: v.already_in_state,
        }
    }
}

impl From<GetQueueMetricsArgs> for domain::metrics::QueueMetricsQuery {
    fn from(v: GetQueueMetricsArgs) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            range: v.range.into(),
            now_ms: v.now_ms,
        }
    }
}

impl From<SummarizeWindowArgs> for domain::timeline::WindowQuery {
    fn from(v: SummarizeWindowArgs) -> Self {
        Self {
            from_ms: v.from_ms,
            to_ms: v.to_ms,
            range_from_ms: v.range_from_ms,
        }
    }
}

/// Flow links between queues, added per minute, oldest waiting.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GetTopologyArgs {
    pub connection_id: String,
}

/// What happened on the connection's queues since the last
/// read (their `…:events` streams), newest first, capped. The overview asks
/// while it is on screen and stops when it is not.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReadQueueEventsArgs {
    pub connection_id: String,
    /// Where each queue's last read stopped; a queue without one is read
    /// from its last few events.
    #[serde(default)]
    pub cursors: Vec<EventCursor>,
    /// Events wanted at most (default 100).
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GetServerInfoArgs {
    pub connection_id: String,
}

/// connection-level overview that aggregates metrics
/// across every queue on the connection. Frontend uses this for the
/// env-level dashboard at `/connections/$id/`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GetConnectionMetricsArgs {
    pub connection_id: String,
    /// The window (5m … 7d). Defaults to 24h.
    #[serde(default)]
    pub range: MetricsRange,
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
    /// Scheduler key: BullMQ's stable identifier (the developer-supplied
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
    /// Iteration count: number of times this scheduler has already
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

impl From<domain::schedule::CronJob> for CronJob {
    fn from(v: domain::schedule::CronJob) -> Self {
        Self {
            key: v.key,
            name: v.name,
            pattern: v.pattern,
            every: v.every,
            tz: v.tz,
            offset: v.offset,
            iteration_count: v.iteration_count,
            next_run: v.next_run,
            end_date: v.end_date,
            template: v.template,
            paused: v.paused,
        }
    }
}

/// What saving a scheduler produced.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerSaved {
    pub scheduler_id: String,
    /// When its pending job runs (ms).
    pub next_run_ms: i64,
    /// Id of that pending job: `repeat:{id}:{next run}`.
    pub job_id: String,
}

impl From<domain::schedule::SchedulerSaved> for SchedulerSaved {
    fn from(v: domain::schedule::SchedulerSaved) -> Self {
        Self {
            scheduler_id: v.scheduler_id,
            next_run_ms: v.next_run_ms,
            job_id: v.job_id,
        }
    }
}

/// One BullMQ worker = one named Redis connection.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueueWorker {
    /// Redis client id (`id=`), unique per connection on that server.
    pub id: String,
    /// The worker's own name (`new Worker(q, fn, { name })`), if it has one.
    pub name: Option<String>,
    /// `ip:port` of the worker process as Redis sees it.
    pub addr: String,
    /// Seconds since the connection was opened.
    pub age_secs: u64,
    /// Seconds since its last command. A healthy idle worker sits in a
    /// blocking pop, so this stays under its `drainDelay` (5 s by default).
    pub idle_secs: u64,
    /// Last command (`cmd=`), e.g. `bzpopmin`.
    pub last_command: Option<String>,
    /// Client library as reported by Redis ≥ 7.2 (`lib-name` / `lib-ver`).
    pub library: Option<String>,
    /// Every `key=value` of the `CLIENT LIST` line, for the detail sheet.
    pub raw: BTreeMap<String, String>,
}

impl From<domain::workers::QueueWorker> for QueueWorker {
    fn from(v: domain::workers::QueueWorker) -> Self {
        Self {
            id: v.id,
            name: v.name,
            addr: v.addr,
            age_secs: v.age_secs,
            idle_secs: v.idle_secs,
            last_command: v.last_command,
            library: v.library,
            raw: v.raw,
        }
    }
}

/// Result of `list_workers`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueueWorkers {
    /// `false` = this Redis refuses `CLIENT LIST`; `workers` is empty and
    /// `unsupported_reason` carries the server's message.
    pub supported: bool,
    pub unsupported_reason: Option<String>,
    pub workers: Vec<QueueWorker>,
}

impl From<domain::workers::QueueWorkers> for QueueWorkers {
    fn from(v: domain::workers::QueueWorkers) -> Self {
        Self {
            supported: v.supported,
            unsupported_reason: v.unsupported_reason,
            workers: v.workers.into_iter().map(Into::into).collect(),
        }
    }
}

/// How many workers one queue has.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueueWorkerCount {
    pub queue_id: QueueId,
    pub workers: u32,
}

impl From<domain::workers::QueueWorkerCount> for QueueWorkerCount {
    fn from(v: domain::workers::QueueWorkerCount) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            workers: v.workers,
        }
    }
}

/// Result of `count_queue_workers`: one `CLIENT LIST` for many queues.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueueWorkerCounts {
    /// `false` = this Redis refuses `CLIENT LIST`: `counts` is empty, and no
    /// queue may be called worker-less from it.
    pub supported: bool,
    pub counts: Vec<QueueWorkerCount>,
}

impl From<domain::workers::QueueWorkerCounts> for QueueWorkerCounts {
    fn from(v: domain::workers::QueueWorkerCounts) -> Self {
        Self {
            supported: v.supported,
            counts: v.counts.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SlowJob {
    pub queue_id: QueueId,
    pub queue_name: String,
    pub job_id: String,
    pub name: String,
    pub processing_time_ms: u64,
    pub finished_on: i64,
}

impl From<domain::metrics::SlowJob> for SlowJob {
    fn from(v: domain::metrics::SlowJob) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            queue_name: v.queue_name,
            job_id: v.job_id,
            name: v.name,
            processing_time_ms: v.processing_time_ms,
            finished_on: v.finished_on,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FailingJobType {
    pub queue_id: QueueId,
    pub queue_name: String,
    pub name: String,
    pub count: u32,
    pub last_failed_at: i64,
    pub last_reason: Option<String>,
}

impl From<domain::metrics::FailingJobType> for FailingJobType {
    fn from(v: domain::metrics::FailingJobType) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            queue_name: v.queue_name,
            name: v.name,
            count: v.count,
            last_failed_at: v.last_failed_at,
            last_reason: v.last_reason,
        }
    }
}

/// Metrics of one queue, or of a whole connection (see [`merge`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueMetrics {
    pub range: MetricsRange,
    pub bucket_ms: u32,
    /// Start (ms) of each bucket: the chart's x axis.
    pub bucket_starts_ms: Vec<f64>,

    pub completed_total: u64,
    pub failed_total: u64,
    /// `failed / (completed + failed)`; `0.0` when nothing finished.
    pub failure_rate: f64,
    /// Completed jobs per minute over the window.
    pub throughput_per_minute: f64,
    pub completed_series: Vec<u64>,
    pub failed_series: Vec<u64>,

    pub avg_processing_time_ms: u64,
    pub min_processing_time_ms: u64,
    pub max_processing_time_ms: u64,
    pub avg_queue_delay_ms: u64,
    pub min_queue_delay_ms: u64,
    pub max_queue_delay_ms: u64,
    /// Per-bucket averages; `0` where no sampled job finished.
    pub processing_time_series_ms: Vec<u64>,
    pub queue_delay_series_ms: Vec<u64>,
    /// How many completed jobs the timing figures are based on.
    pub sampled_jobs: u32,

    /// Queues whose counts come from BullMQ's native metrics, out of
    /// `total_queues`. `1 / 1` or `0 / 1` for a single queue.
    pub native_queues: u32,
    pub total_queues: u32,

    pub slowest_jobs: Vec<SlowJob>,
    pub failing_job_types: Vec<FailingJobType>,
    /// The median processing time of each job type over the
    /// window (completed jobs of the sample): what "usual" means on the
    /// Active tab.
    #[serde(default)]
    pub usual_durations: Vec<UsualDuration>,
}

impl From<domain::metrics::QueueMetrics> for QueueMetrics {
    fn from(v: domain::metrics::QueueMetrics) -> Self {
        Self {
            range: v.range.into(),
            bucket_ms: v.bucket_ms,
            bucket_starts_ms: v.bucket_starts_ms,
            completed_total: v.completed_total,
            failed_total: v.failed_total,
            failure_rate: v.failure_rate,
            throughput_per_minute: v.throughput_per_minute,
            completed_series: v.completed_series,
            failed_series: v.failed_series,
            avg_processing_time_ms: v.avg_processing_time_ms,
            min_processing_time_ms: v.min_processing_time_ms,
            max_processing_time_ms: v.max_processing_time_ms,
            avg_queue_delay_ms: v.avg_queue_delay_ms,
            min_queue_delay_ms: v.min_queue_delay_ms,
            max_queue_delay_ms: v.max_queue_delay_ms,
            processing_time_series_ms: v.processing_time_series_ms,
            queue_delay_series_ms: v.queue_delay_series_ms,
            sampled_jobs: v.sampled_jobs,
            native_queues: v.native_queues,
            total_queues: v.total_queues,
            slowest_jobs: v.slowest_jobs.into_iter().map(Into::into).collect(),
            failing_job_types: v.failing_job_types.into_iter().map(Into::into).collect(),
            usual_durations: v.usual_durations.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsualDuration {
    pub queue_name: String,
    pub name: String,
    pub median_ms: u64,
    /// Completed jobs the median is taken over.
    pub samples: u32,
}

impl From<domain::metrics::UsualDuration> for UsualDuration {
    fn from(v: domain::metrics::UsualDuration) -> Self {
        Self {
            queue_name: v.queue_name,
            name: v.name,
            median_ms: v.median_ms,
            samples: v.samples,
        }
    }
}

/// Completed / failed counts over time for every queue of a
/// connection, from BullMQ's native per-minute metrics only: cheap
/// enough to draw a heat strip per queue. A queue that records no metrics
/// says so (`metrics_off`) instead of drawing zeros.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Throughput {
    pub range: MetricsRange,
    pub bucket_ms: u32,
    pub bucket_starts_ms: Vec<f64>,
    pub queues: Vec<QueueThroughput>,
    /// Sum over the queues that record metrics.
    pub completed: Vec<u64>,
    pub failed: Vec<u64>,
    /// Queues without native metrics, out of `queues.len()`.
    pub metrics_off_queues: u32,
}

impl From<domain::metrics::Throughput> for Throughput {
    fn from(v: domain::metrics::Throughput) -> Self {
        Self {
            range: v.range.into(),
            bucket_ms: v.bucket_ms,
            bucket_starts_ms: v.bucket_starts_ms,
            queues: v.queues.into_iter().map(Into::into).collect(),
            completed: v.completed,
            failed: v.failed,
            metrics_off_queues: v.metrics_off_queues,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueThroughput {
    pub queue_id: QueueId,
    pub queue_name: String,
    /// BullMQ's metrics are not recorded for this queue (the worker option
    /// `metrics` is off): the series are empty, not zeros.
    pub metrics_off: bool,
    pub completed: Vec<u64>,
    pub failed: Vec<u64>,
}

impl From<domain::metrics::QueueThroughput> for QueueThroughput {
    fn from(v: domain::metrics::QueueThroughput) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            queue_name: v.queue_name,
            metrics_off: v.metrics_off,
            completed: v.completed,
            failed: v.failed,
        }
    }
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

impl From<domain::timeline::WindowCause> for WindowCause {
    fn from(v: domain::timeline::WindowCause) -> Self {
        Self {
            signature: v.signature,
            sample: v.sample,
            count: v.count,
            jobs: v.jobs.into_iter().map(Into::into).collect(),
        }
    }
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

impl From<domain::timeline::WindowSummary> for WindowSummary {
    fn from(v: domain::timeline::WindowSummary) -> Self {
        Self {
            from_ms: v.from_ms,
            to_ms: v.to_ms,
            done: v.done,
            failed: v.failed,
            usual_done: v.usual_done,
            usual_failed: v.usual_failed,
            wait_p95_ms: v.wait_p95_ms,
            usual_wait_p95_ms: v.usual_wait_p95_ms,
            causes: v.causes.into_iter().map(Into::into).collect(),
            scanned: v.scanned,
            scan_capped: v.scan_capped,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowLink {
    pub parent: QueueId,
    pub child: QueueId,
    /// Child jobs seen on the link (among the newest parents).
    pub jobs: u32,
}

impl From<domain::topology::FlowLink> for FlowLink {
    fn from(v: domain::topology::FlowLink) -> Self {
        Self {
            parent: v.parent.into(),
            child: v.child.into(),
            jobs: v.jobs,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueFlowStats {
    pub queue_id: QueueId,
    /// `added` events per minute over the last hour.
    pub added_per_minute: f64,
    /// The stream did not reach back an hour: a lower bound.
    pub added_capped: bool,
    /// When the oldest waiting job was added (ms); `None` = nothing waits.
    pub oldest_waiting_ms: Option<i64>,
}

impl From<domain::topology::QueueFlowStats> for QueueFlowStats {
    fn from(v: domain::topology::QueueFlowStats) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            added_per_minute: v.added_per_minute,
            added_capped: v.added_capped,
            oldest_waiting_ms: v.oldest_waiting_ms,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Topology {
    pub links: Vec<FlowLink>,
    pub queues: Vec<QueueFlowStats>,
}

impl From<domain::topology::Topology> for Topology {
    fn from(v: domain::topology::Topology) -> Self {
        Self {
            links: v.links.into_iter().map(Into::into).collect(),
            queues: v.queues.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueEvent {
    pub queue_id: QueueId,
    pub queue_name: String,
    /// The stream id (`<ms>-<seq>`), unique within the queue.
    pub stream_id: String,
    pub at_ms: f64,
    /// `added`, `active`, `completed`, `failed`, `delayed`, `stalled`,
    /// `removed`, or `retried` (BullMQ's `waiting` after a failure).
    pub event: String,
    pub job_id: Option<String>,
    pub job_name: Option<String>,
    pub failed_reason: Option<String>,
}

impl From<domain::events::QueueEvent> for QueueEvent {
    fn from(v: domain::events::QueueEvent) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            queue_name: v.queue_name,
            stream_id: v.stream_id,
            at_ms: v.at_ms,
            event: v.event,
            job_id: v.job_id,
            job_name: v.job_name,
            failed_reason: v.failed_reason,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueEvents {
    /// Newest first.
    pub events: Vec<QueueEvent>,
    /// Where to read from next time, one per queue read.
    pub cursors: Vec<EventCursor>,
    /// More happened than was read: the newest were kept.
    pub sampled: bool,
}

impl From<domain::events::QueueEvents> for QueueEvents {
    fn from(v: domain::events::QueueEvents) -> Self {
        Self {
            events: v.events.into_iter().map(Into::into).collect(),
            cursors: v.cursors.into_iter().map(Into::into).collect(),
            sampled: v.sampled,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub redis_version: Option<String>,
    /// Round trip of one `PING`, ms.
    pub latency_ms: u32,
    pub used_memory_bytes: Option<f64>,
    /// `maxmemory`; `None` when unlimited (0) or not reported.
    pub max_memory_bytes: Option<f64>,
    pub ops_per_sec: Option<f64>,
    pub connected_clients: Option<f64>,
}

impl From<domain::server::ServerInfo> for ServerInfo {
    fn from(v: domain::server::ServerInfo) -> Self {
        Self {
            redis_version: v.redis_version,
            latency_ms: v.latency_ms,
            used_memory_bytes: v.used_memory_bytes,
            max_memory_bytes: v.max_memory_bytes,
            ops_per_sec: v.ops_per_sec,
            connected_clients: v.connected_clients,
        }
    }
}

/// What a maintenance operation would touch, shown before confirming.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MaintenancePreview {
    /// Jobs the operation will act on. For drain this is an upper bound:
    /// flow members are only recognised job by job, while running.
    pub affected: u64,
    /// Drain only: pending scheduler runs that will be left in place.
    pub spared_scheduler_jobs: u64,
}

impl From<domain::maintenance::MaintenancePreview> for MaintenancePreview {
    fn from(v: domain::maintenance::MaintenancePreview) -> Self {
        Self {
            affected: v.affected,
            spared_scheduler_jobs: v.spared_scheduler_jobs,
        }
    }
}

/// Outcome of a maintenance run.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
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

impl From<domain::maintenance::MaintenanceResult> for MaintenanceResult {
    fn from(v: domain::maintenance::MaintenanceResult) -> Self {
        Self {
            action: v.action,
            affected: v.affected,
            skipped: v.skipped,
            cancelled: v.cancelled,
        }
    }
}

/// The selectable windows: eight, from 5 minutes to 7 days.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, Type, PartialEq, Eq, Hash)]
pub enum MetricsRange {
    #[serde(rename = "5m")]
    M5,
    #[serde(rename = "15m")]
    M15,
    #[serde(rename = "30m")]
    M30,
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "6h")]
    H6,
    /// The default window.
    #[default]
    #[serde(rename = "24h")]
    H24,
    #[serde(rename = "3d")]
    D3,
    #[serde(rename = "7d")]
    D7,
}

impl From<domain::metrics::MetricsRange> for MetricsRange {
    fn from(v: domain::metrics::MetricsRange) -> Self {
        match v {
            domain::metrics::MetricsRange::M5 => Self::M5,
            domain::metrics::MetricsRange::M15 => Self::M15,
            domain::metrics::MetricsRange::M30 => Self::M30,
            domain::metrics::MetricsRange::H1 => Self::H1,
            domain::metrics::MetricsRange::H6 => Self::H6,
            domain::metrics::MetricsRange::H24 => Self::H24,
            domain::metrics::MetricsRange::D3 => Self::D3,
            domain::metrics::MetricsRange::D7 => Self::D7,
        }
    }
}

impl From<MetricsRange> for domain::metrics::MetricsRange {
    fn from(v: MetricsRange) -> Self {
        match v {
            MetricsRange::M5 => Self::M5,
            MetricsRange::M15 => Self::M15,
            MetricsRange::M30 => Self::M30,
            MetricsRange::H1 => Self::H1,
            MetricsRange::H6 => Self::H6,
            MetricsRange::H24 => Self::H24,
            MetricsRange::D3 => Self::D3,
            MetricsRange::D7 => Self::D7,
        }
    }
}

/// Where the last read of a queue stopped (a stream id).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EventCursor {
    pub queue_id: QueueId,
    pub last_id: String,
}

impl From<domain::events::EventCursor> for EventCursor {
    fn from(v: domain::events::EventCursor) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            last_id: v.last_id,
        }
    }
}

impl From<EventCursor> for domain::events::EventCursor {
    fn from(v: EventCursor) -> Self {
        Self {
            queue_id: v.queue_id.into(),
            last_id: v.last_id,
        }
    }
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
    /// and, unless `include_delayed` is false, delayed.
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
impl From<domain::maintenance::CleanState> for CleanState {
    fn from(v: domain::maintenance::CleanState) -> Self {
        match v {
            domain::maintenance::CleanState::Completed => Self::Completed,
            domain::maintenance::CleanState::Failed => Self::Failed,
        }
    }
}

impl From<CleanState> for domain::maintenance::CleanState {
    fn from(v: CleanState) -> Self {
        match v {
            CleanState::Completed => Self::Completed,
            CleanState::Failed => Self::Failed,
        }
    }
}

impl From<domain::maintenance::QueueMaintenance> for QueueMaintenance {
    fn from(v: domain::maintenance::QueueMaintenance) -> Self {
        match v {
            domain::maintenance::QueueMaintenance::Drain { include_delayed } => {
                Self::Drain { include_delayed }
            }
            domain::maintenance::QueueMaintenance::Clean {
                state,
                older_than_ms,
            } => Self::Clean {
                state: state.into(),
                older_than_ms,
            },
            domain::maintenance::QueueMaintenance::RetryAllFailed => Self::RetryAllFailed,
        }
    }
}

impl From<QueueMaintenance> for domain::maintenance::QueueMaintenance {
    fn from(v: QueueMaintenance) -> Self {
        match v {
            QueueMaintenance::Drain { include_delayed } => Self::Drain { include_delayed },
            QueueMaintenance::Clean {
                state,
                older_than_ms,
            } => Self::Clean {
                state: state.into(),
                older_than_ms,
            },
            QueueMaintenance::RetryAllFailed => Self::RetryAllFailed,
        }
    }
}

/// When a scheduler fires.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Schedule {
    /// A cron expression, 5 fields (minute first) or 6 (second first), read
    /// in `tz` (IANA name), UTC when `None`.
    #[serde(rename_all = "camelCase")]
    Pattern { pattern: String, tz: Option<String> },
    /// A fixed interval.
    #[serde(rename_all = "camelCase")]
    Every { every_ms: u64 },
}

/// A scheduler as the user (or an agent) describes it.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerSpec {
    /// The scheduler id: BullMQ's key. Saving an existing id edits it.
    pub id: String,
    /// Name of the jobs it produces.
    pub job_name: String,
    pub schedule: Schedule,
    /// JSON template: the `data` of every job produced. `{}` when empty.
    pub data: String,
}

impl From<domain::schedule::Schedule> for Schedule {
    fn from(v: domain::schedule::Schedule) -> Self {
        match v {
            domain::schedule::Schedule::Pattern { pattern, tz } => Self::Pattern { pattern, tz },
            domain::schedule::Schedule::Every { every_ms } => Self::Every { every_ms },
        }
    }
}

impl From<Schedule> for domain::schedule::Schedule {
    fn from(v: Schedule) -> Self {
        match v {
            Schedule::Pattern { pattern, tz } => Self::Pattern { pattern, tz },
            Schedule::Every { every_ms } => Self::Every { every_ms },
        }
    }
}

impl From<SchedulerSpec> for domain::schedule::SchedulerSpec {
    fn from(v: SchedulerSpec) -> Self {
        Self {
            id: v.id,
            job_name: v.job_name,
            schedule: v.schedule.into(),
            data: v.data,
        }
    }
}

impl From<domain::schedule::SchedulerSpec> for SchedulerSpec {
    fn from(v: domain::schedule::SchedulerSpec) -> Self {
        Self {
            id: v.id,
            job_name: v.job_name,
            schedule: v.schedule.into(),
            data: v.data,
        }
    }
}

/// A queue, as the listing named it. Opaque: take it from a listing (or a
/// URL that came from one) and pass it back; never build one or take it
/// apart. The BullMQ backend makes `{prefix}:{name}`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct QueueId(pub String);

impl QueueId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for QueueId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<domain::queues::QueueId> for QueueId {
    fn from(v: domain::queues::QueueId) -> Self {
        Self(v.as_str().to_string())
    }
}

impl From<QueueId> for domain::queues::QueueId {
    fn from(v: QueueId) -> Self {
        Self::new(v.0)
    }
}

impl From<&QueueId> for domain::queues::QueueId {
    fn from(v: &QueueId) -> Self {
        Self::new(v.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_range_crosses_the_wire_by_its_short_name() {
        assert_eq!(
            serde_json::to_string(&MetricsRange::H24).unwrap(),
            "\"24h\""
        );
        assert_eq!(
            serde_json::from_str::<MetricsRange>("\"5m\"").unwrap(),
            MetricsRange::M5
        );
    }

    // CronJob round-trips through the camelCase wire shape.
    #[test]
    fn cron_job_serializes_to_camel_case_wire_shape() {
        let cj = CronJob::from(domain::schedule::CronJob {
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
        });
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
