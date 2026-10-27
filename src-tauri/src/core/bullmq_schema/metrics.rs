//! Queue metrics.
//!
//! Pure types and arithmetic: time ranges and their buckets, turning BullMQ's
//! native per-minute metrics into a series, folding a sample of finished jobs
//! into timing figures / slowest jobs / failing job types, and merging the
//! metrics of several queues. The Redis round-trips live in the connection
//! actor; everything here is unit-tested without Redis.
//!
//! ## Two sources for the counts
//!
//! - **Native** — a BullMQ `Worker` created with the `metrics` option keeps
//!   per-minute counters in `{q}:metrics:completed|failed` (+ `:data`). They
//!   survive `removeOnComplete`, so they are exact.
//! - **Estimated** — otherwise the counts are `ZCOUNT`s over the `completed` /
//!   `failed` sets, i.e. only the jobs *still in Redis*. With
//!   `removeOnComplete` this undercounts; the UI says so.
//!
//! Timing figures, slowest jobs and failing types always come from a bounded
//! sample of job hashes — native metrics carry no durations.

use serde::{Deserialize, Serialize};
use specta::Type;

const MINUTE_MS: i64 = 60_000;
/// Rows of the "slowest jobs" and "most failing job types" tables.
pub const TOP_N: usize = 10;

/// The selectable windows — the same eight as BullStudio.
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

impl MetricsRange {
    /// `(bucket size in minutes, bucket count)`. The size is always a whole
    /// number of minutes so native per-minute points fold in exactly.
    fn plan(self) -> (i64, usize) {
        match self {
            Self::M5 => (1, 5),
            Self::M15 => (1, 15),
            Self::M30 => (2, 15),
            Self::H1 => (5, 12),
            Self::H6 => (30, 12),
            Self::H24 => (60, 24),
            Self::D3 => (180, 24),
            Self::D7 => (360, 28),
        }
    }

    pub fn bucket_ms(self) -> i64 {
        self.plan().0 * MINUTE_MS
    }

    pub fn bucket_count(self) -> usize {
        self.plan().1
    }

    pub fn window_ms(self) -> i64 {
        self.bucket_ms() * self.bucket_count() as i64
    }

    pub fn window_minutes(self) -> i64 {
        self.window_ms() / MINUTE_MS
    }
}

/// The buckets of a range, anchored on `now`: the last bucket ends at `now`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Buckets {
    pub range: MetricsRange,
    pub now_ms: i64,
}

impl Buckets {
    pub fn new(range: MetricsRange, now_ms: i64) -> Self {
        Self { range, now_ms }
    }

    pub fn window_start_ms(&self) -> i64 {
        self.now_ms - self.range.window_ms()
    }

    pub fn count(&self) -> usize {
        self.range.bucket_count()
    }

    /// Start of bucket `i` (inclusive-exclusive lower edge: `(start, end]`).
    pub fn start_ms(&self, i: usize) -> i64 {
        self.window_start_ms() + i as i64 * self.range.bucket_ms()
    }

    pub fn starts_ms(&self) -> Vec<i64> {
        (0..self.count()).map(|i| self.start_ms(i)).collect()
    }

    /// `(lo, hi)` bounds for a `ZCOUNT key lo hi` covering bucket `i`.
    pub fn zcount_bounds(&self, i: usize) -> (i64, i64) {
        (
            self.start_ms(i) + 1,
            self.start_ms(i) + self.range.bucket_ms(),
        )
    }

    /// Bucket of a timestamp, `None` outside the window. A timestamp slightly
    /// in the future (clock skew with the worker host) counts as "now".
    pub fn index_of(&self, ts_ms: i64) -> Option<usize> {
        let ts = ts_ms.min(self.now_ms);
        let offset = ts - self.window_start_ms();
        if offset <= 0 {
            return None;
        }
        let idx = ((offset - 1) / self.range.bucket_ms()) as usize;
        Some(idx.min(self.count() - 1))
    }
}

/// What BullMQ keeps per metric type (`completed` / `failed`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeMetrics {
    /// `HGET …:metrics:<type> count` — all-time total.
    pub count: u64,
    /// `prevCount` — total at the last minute rollover.
    pub prev_count: u64,
    /// `prevTS` (ms) — when the last rollover happened.
    pub prev_ts_ms: i64,
    /// `LRANGE …:metrics:<type>:data 0 N` — per-minute counts, most recent
    /// first. `data[i]` is the minute `floor(prevTS / 1min) - 1 - i`.
    pub data: Vec<u64>,
}

impl NativeMetrics {
    /// BullStudio's rule: a queue is metric-backed once anything was counted.
    pub fn is_recording(&self) -> bool {
        self.count > 0
    }

    /// Fold the per-minute points into the buckets.
    ///
    /// BullMQ's `collectMetrics` pushes, at each minute rollover, the count of
    /// the minute that just ended, then zeros for the silent minutes in
    /// between — so the list head is always the minute *before* `prevTS`'s.
    /// The minute in progress is `count - prevCount`, not in the list yet.
    /// Minutes after `prevTS` have seen nothing (or the hash would have moved).
    pub fn series(&self, buckets: &Buckets) -> Vec<u64> {
        let mut series = vec![0u64; buckets.count()];
        if !self.is_recording() {
            return series;
        }
        let prev_minute = self.prev_ts_ms.div_euclid(MINUTE_MS);
        let mut add = |minute: i64, value: u64| {
            // Attribute a minute to the bucket holding its END — `(start, end]`.
            if let Some(i) = buckets.index_of((minute + 1) * MINUTE_MS) {
                // …unless the minute ends after `now`: it is the running one.
                series[i] += value;
            }
        };
        add(prev_minute, self.count.saturating_sub(self.prev_count));
        for (i, value) in self.data.iter().enumerate() {
            add(prev_minute - 1 - i as i64, *value);
        }
        series
    }
}

/// One finished job, as far as metrics care. `None` = field absent / invalid.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobSample {
    pub job_id: String,
    pub name: String,
    pub timestamp: Option<i64>,
    pub processed_on: Option<i64>,
    pub finished_on: Option<i64>,
    /// `opts.delay` (ms) — not part of the queue delay the job *suffered*.
    pub delay: Option<i64>,
    pub failed_reason: Option<String>,
}

impl JobSample {
    pub fn processing_time_ms(&self) -> Option<u64> {
        match (self.processed_on, self.finished_on) {
            (Some(p), Some(f)) if f >= p => Some((f - p) as u64),
            _ => None,
        }
    }

    /// Time spent waiting for a worker: `processedOn − timestamp − delay`.
    pub fn queue_delay_ms(&self) -> Option<u64> {
        match (self.timestamp, self.processed_on) {
            (Some(t), Some(p)) => {
                let waited = p - t - self.delay.unwrap_or(0).max(0);
                (waited >= 0).then_some(waited as u64)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SlowJob {
    pub queue_name: String,
    pub job_id: String,
    pub name: String,
    pub processing_time_ms: u64,
    pub finished_on: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FailingJobType {
    pub queue_name: String,
    pub name: String,
    pub count: u32,
    pub last_failed_at: i64,
    pub last_reason: Option<String>,
}

/// Metrics of one queue — or of a whole connection (see [`merge`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueMetrics {
    pub range: MetricsRange,
    pub bucket_ms: u32,
    /// Start (ms) of each bucket — the chart's x axis.
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

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsualDuration {
    pub queue_name: String,
    pub name: String,
    pub median_ms: u64,
    /// Completed jobs the median is taken over.
    pub samples: u32,
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

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueThroughput {
    pub queue_ref: super::domain::QueueRef,
    /// BullMQ's metrics are not recorded for this queue (the worker option
    /// `metrics` is off): the series are empty, not zeros.
    pub metrics_off: bool,
    pub completed: Vec<u64>,
    pub failed: Vec<u64>,
}

/// Fold each queue's native metrics into the range's buckets and sum them.
pub fn throughput(
    range: MetricsRange,
    now_ms: i64,
    queues: Vec<(super::domain::QueueRef, NativeMetrics, NativeMetrics)>,
) -> Throughput {
    let buckets = Buckets::new(range, now_ms);
    let n = buckets.count();
    let mut out = Throughput {
        range,
        bucket_ms: range.bucket_ms() as u32,
        bucket_starts_ms: buckets.starts_ms().into_iter().map(|v| v as f64).collect(),
        completed: vec![0; n],
        failed: vec![0; n],
        ..Default::default()
    };
    for (queue_ref, completed, failed) in queues {
        let off = !completed.is_recording() && !failed.is_recording();
        if off {
            out.metrics_off_queues += 1;
            out.queues.push(QueueThroughput {
                queue_ref,
                metrics_off: true,
                completed: Vec::new(),
                failed: Vec::new(),
            });
            continue;
        }
        let c = completed.series(&buckets);
        let f = failed.series(&buckets);
        for i in 0..n {
            out.completed[i] += c[i];
            out.failed[i] += f[i];
        }
        out.queues.push(QueueThroughput {
            queue_ref,
            metrics_off: false,
            completed: c,
            failed: f,
        });
    }
    out
}

/// Median processing time per job name, most sampled first.
pub fn usual_durations(queue_name: &str, by_name: Vec<(String, Vec<u64>)>) -> Vec<UsualDuration> {
    let mut out: Vec<UsualDuration> = by_name
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(name, mut v)| {
            v.sort_unstable();
            let mid = v.len() / 2;
            let median_ms = if v.len() % 2 == 0 {
                (v[mid - 1] + v[mid]) / 2
            } else {
                v[mid]
            };
            UsualDuration {
                queue_name: queue_name.to_string(),
                name,
                median_ms,
                samples: v.len() as u32,
            }
        })
        .collect();
    out.sort_by(|a, b| b.samples.cmp(&a.samples).then(a.name.cmp(&b.name)));
    out
}

fn failure_rate(completed: u64, failed: u64) -> f64 {
    let total = completed + failed;
    if total == 0 {
        0.0
    } else {
        failed as f64 / total as f64
    }
}

#[derive(Default, Clone, Copy)]
struct Stat {
    sum: u128,
    n: u128,
    min: Option<u64>,
    max: u64,
}

impl Stat {
    fn push(&mut self, v: u64) {
        self.sum += v as u128;
        self.n += 1;
        self.min = Some(self.min.map_or(v, |m| m.min(v)));
        self.max = self.max.max(v);
    }

    fn avg(&self) -> u64 {
        self.sum.checked_div(self.n).unwrap_or(0) as u64
    }
}

/// Everything known about one queue, ready to be turned into [`QueueMetrics`].
pub struct QueueMetricsInput<'a> {
    pub queue_name: &'a str,
    pub buckets: &'a Buckets,
    /// `Some` when the queue records native metrics; else the ZCOUNT series.
    pub native_completed: Option<&'a NativeMetrics>,
    pub native_failed: Option<&'a NativeMetrics>,
    pub estimated_completed: &'a [u64],
    pub estimated_failed: &'a [u64],
    /// Most recent completed jobs in the window (bounded sample).
    pub completed_sample: &'a [JobSample],
    /// Most recent failed jobs in the window (bounded sample).
    pub failed_sample: &'a [JobSample],
}

pub fn build(input: QueueMetricsInput<'_>) -> QueueMetrics {
    let buckets = input.buckets;
    let n = buckets.count();
    let native = input
        .native_completed
        .is_some_and(NativeMetrics::is_recording)
        || input.native_failed.is_some_and(NativeMetrics::is_recording);

    let pick = |native_m: Option<&NativeMetrics>, estimated: &[u64]| -> Vec<u64> {
        if native {
            native_m
                .map(|m| m.series(buckets))
                .unwrap_or_else(|| vec![0; n])
        } else {
            let mut s = estimated.to_vec();
            s.resize(n, 0);
            s
        }
    };
    let completed_series = pick(input.native_completed, input.estimated_completed);
    let failed_series = pick(input.native_failed, input.estimated_failed);
    let completed_total: u64 = completed_series.iter().sum();
    let failed_total: u64 = failed_series.iter().sum();

    // Timing, from the sample.
    let mut pt = Stat::default();
    let mut qd = Stat::default();
    let mut pt_buckets = vec![Stat::default(); n];
    let mut qd_buckets = vec![Stat::default(); n];
    let mut slowest: Vec<SlowJob> = Vec::new();
    let mut by_name: Vec<(String, Vec<u64>)> = Vec::new();
    let mut sampled = 0u32;
    for job in input.completed_sample {
        let Some(finished_on) = job.finished_on else {
            continue;
        };
        let Some(bucket) = buckets.index_of(finished_on) else {
            continue;
        };
        sampled += 1;
        if let Some(v) = job.processing_time_ms() {
            pt.push(v);
            pt_buckets[bucket].push(v);
            match by_name.iter_mut().find(|(n, _)| n == &job.name) {
                Some((_, durations)) => durations.push(v),
                None => by_name.push((job.name.clone(), vec![v])),
            }
            slowest.push(SlowJob {
                queue_name: input.queue_name.to_string(),
                job_id: job.job_id.clone(),
                name: job.name.clone(),
                processing_time_ms: v,
                finished_on,
            });
        }
        if let Some(v) = job.queue_delay_ms() {
            qd.push(v);
            qd_buckets[bucket].push(v);
        }
    }
    sort_slowest(&mut slowest);

    // Failing job types, from the failed sample.
    let mut types: Vec<FailingJobType> = Vec::new();
    for job in input.failed_sample {
        let Some(finished_on) = job.finished_on else {
            continue;
        };
        if buckets.index_of(finished_on).is_none() {
            continue;
        }
        match types.iter_mut().find(|t| t.name == job.name) {
            Some(t) => {
                t.count += 1;
                if finished_on > t.last_failed_at {
                    t.last_failed_at = finished_on;
                    t.last_reason = job.failed_reason.clone();
                }
            }
            None => types.push(FailingJobType {
                queue_name: input.queue_name.to_string(),
                name: job.name.clone(),
                count: 1,
                last_failed_at: finished_on,
                last_reason: job.failed_reason.clone(),
            }),
        }
    }
    sort_failing(&mut types);

    QueueMetrics {
        range: buckets.range,
        bucket_ms: buckets.range.bucket_ms() as u32,
        bucket_starts_ms: buckets.starts_ms().into_iter().map(|v| v as f64).collect(),
        completed_total,
        failed_total,
        failure_rate: failure_rate(completed_total, failed_total),
        throughput_per_minute: completed_total as f64 / buckets.range.window_minutes() as f64,
        completed_series,
        failed_series,
        avg_processing_time_ms: pt.avg(),
        min_processing_time_ms: pt.min.unwrap_or(0),
        max_processing_time_ms: pt.max,
        avg_queue_delay_ms: qd.avg(),
        min_queue_delay_ms: qd.min.unwrap_or(0),
        max_queue_delay_ms: qd.max,
        processing_time_series_ms: pt_buckets.iter().map(Stat::avg).collect(),
        queue_delay_series_ms: qd_buckets.iter().map(Stat::avg).collect(),
        sampled_jobs: sampled,
        native_queues: native as u32,
        total_queues: 1,
        slowest_jobs: slowest,
        failing_job_types: types,
        usual_durations: usual_durations(input.queue_name, by_name),
    }
}

fn sort_slowest(jobs: &mut Vec<SlowJob>) {
    jobs.sort_by(|a, b| {
        b.processing_time_ms
            .cmp(&a.processing_time_ms)
            .then(b.finished_on.cmp(&a.finished_on))
    });
    jobs.truncate(TOP_N);
}

fn sort_failing(types: &mut Vec<FailingJobType>) {
    types.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then(b.last_failed_at.cmp(&a.last_failed_at))
    });
    types.truncate(TOP_N);
}

/// Connection-level metrics: the queues of a connection, merged. All inputs
/// must share the same range and `now` (the caller builds them in one go).
/// Averages are weighted by each queue's sample size.
pub fn merge(range: MetricsRange, now_ms: i64, queues: &[QueueMetrics]) -> QueueMetrics {
    let buckets = Buckets::new(range, now_ms);
    let n = buckets.count();
    let mut out = QueueMetrics {
        range,
        bucket_ms: range.bucket_ms() as u32,
        bucket_starts_ms: buckets.starts_ms().into_iter().map(|v| v as f64).collect(),
        completed_series: vec![0; n],
        failed_series: vec![0; n],
        processing_time_series_ms: vec![0; n],
        queue_delay_series_ms: vec![0; n],
        ..Default::default()
    };
    if queues.is_empty() {
        return out;
    }

    let weighted = |pick: fn(&QueueMetrics) -> u64| -> u64 {
        let (sum, w) = queues.iter().fold((0u128, 0u128), |(s, w), q| {
            let weight = q.sampled_jobs as u128;
            (s + pick(q) as u128 * weight, w + weight)
        });
        sum.checked_div(w).unwrap_or(0) as u64
    };
    let min_of = |pick: fn(&QueueMetrics) -> u64| -> u64 {
        queues
            .iter()
            .filter(|q| q.sampled_jobs > 0)
            .map(pick)
            .min()
            .unwrap_or(0)
    };

    let mut pt_sum = vec![0u128; n];
    let mut pt_w = vec![0u128; n];
    let mut qd_sum = vec![0u128; n];
    let mut qd_w = vec![0u128; n];
    for q in queues {
        out.completed_total += q.completed_total;
        out.failed_total += q.failed_total;
        out.sampled_jobs += q.sampled_jobs;
        out.native_queues += q.native_queues;
        out.total_queues += q.total_queues;
        for i in 0..n {
            let completed = q.completed_series.get(i).copied().unwrap_or(0);
            out.completed_series[i] += completed;
            out.failed_series[i] += q.failed_series.get(i).copied().unwrap_or(0);
            // Per-bucket averages weighted by that bucket's completed count
            // (at least 1 where the queue has a figure, so it is not lost).
            let pt = q.processing_time_series_ms.get(i).copied().unwrap_or(0);
            if pt > 0 {
                let w = completed.max(1) as u128;
                pt_sum[i] += pt as u128 * w;
                pt_w[i] += w;
            }
            let qd = q.queue_delay_series_ms.get(i).copied().unwrap_or(0);
            if qd > 0 {
                let w = completed.max(1) as u128;
                qd_sum[i] += qd as u128 * w;
                qd_w[i] += w;
            }
        }
        out.slowest_jobs.extend(q.slowest_jobs.iter().cloned());
        out.failing_job_types
            .extend(q.failing_job_types.iter().cloned());
        out.usual_durations
            .extend(q.usual_durations.iter().cloned());
    }
    for i in 0..n {
        out.processing_time_series_ms[i] = pt_sum[i].checked_div(pt_w[i]).unwrap_or(0) as u64;
        out.queue_delay_series_ms[i] = qd_sum[i].checked_div(qd_w[i]).unwrap_or(0) as u64;
    }
    out.failure_rate = failure_rate(out.completed_total, out.failed_total);
    out.throughput_per_minute = out.completed_total as f64 / range.window_minutes() as f64;
    out.avg_processing_time_ms = weighted(|q| q.avg_processing_time_ms);
    out.avg_queue_delay_ms = weighted(|q| q.avg_queue_delay_ms);
    out.min_processing_time_ms = min_of(|q| q.min_processing_time_ms);
    out.min_queue_delay_ms = min_of(|q| q.min_queue_delay_ms);
    out.max_processing_time_ms = queues
        .iter()
        .map(|q| q.max_processing_time_ms)
        .max()
        .unwrap_or(0);
    out.max_queue_delay_ms = queues
        .iter()
        .map(|q| q.max_queue_delay_ms)
        .max()
        .unwrap_or(0);
    sort_slowest(&mut out.slowest_jobs);
    sort_failing(&mut out.failing_job_types);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 12:00:30 on some day — half a minute into a minute, so "the running
    /// minute" is a real thing in these tests.
    const NOW: i64 = 1_800_000_000_000 + 30_000;

    fn minute_of(ts: i64) -> i64 {
        ts.div_euclid(MINUTE_MS)
    }

    #[test]
    fn every_range_is_a_whole_number_of_minute_buckets() {
        for (range, minutes) in [
            (MetricsRange::M5, 5),
            (MetricsRange::M15, 15),
            (MetricsRange::M30, 30),
            (MetricsRange::H1, 60),
            (MetricsRange::H6, 360),
            (MetricsRange::H24, 1440),
            (MetricsRange::D3, 4320),
            (MetricsRange::D7, 10080),
        ] {
            assert_eq!(range.window_minutes(), minutes, "{range:?}");
            assert_eq!(range.bucket_ms() % MINUTE_MS, 0, "{range:?}");
            assert!(
                range.bucket_count() <= 28,
                "{range:?} would be an unreadable chart"
            );
        }
        assert_eq!(
            serde_json::to_string(&MetricsRange::H24).unwrap(),
            "\"24h\""
        );
        assert_eq!(
            serde_json::from_str::<MetricsRange>("\"5m\"").unwrap(),
            MetricsRange::M5
        );
        assert_eq!(MetricsRange::default(), MetricsRange::H24);
    }

    #[test]
    fn buckets_tile_the_window_and_end_at_now() {
        let b = Buckets::new(MetricsRange::H1, NOW);
        assert_eq!(b.count(), 12);
        assert_eq!(b.start_ms(0), NOW - 3_600_000);
        assert_eq!(b.zcount_bounds(11).1, NOW, "last bucket ends at now");
        // Contiguous, non-overlapping ZCOUNT bounds.
        for i in 1..b.count() {
            assert_eq!(b.zcount_bounds(i).0, b.zcount_bounds(i - 1).1 + 1);
        }
        assert_eq!(b.index_of(NOW), Some(11));
        assert_eq!(
            b.index_of(NOW + 5_000),
            Some(11),
            "clock skew counts as now"
        );
        assert_eq!(b.index_of(b.start_ms(0) + 1), Some(0));
        assert_eq!(b.index_of(b.start_ms(0)), None, "lower edge is exclusive");
        assert_eq!(b.index_of(NOW - 7_200_000), None);
        // A timestamp and its ZCOUNT bucket agree.
        let ts = b.start_ms(4) + 17;
        let (lo, hi) = b.zcount_bounds(b.index_of(ts).unwrap());
        assert!(lo <= ts && ts <= hi);
    }

    #[test]
    fn native_points_land_in_the_right_minutes() {
        // Last rollover 2 minutes ago; since then 4 jobs in the running minute
        // of that rollover. The list: 7 jobs the minute before, 0, then 3.
        let prev_ts = NOW - 2 * MINUTE_MS;
        let native = NativeMetrics {
            count: 114,
            prev_count: 110,
            prev_ts_ms: prev_ts,
            data: vec![7, 0, 3],
        };
        let b = Buckets::new(MetricsRange::M5, NOW);
        let series = native.series(&b);
        // M5 = five 1-minute buckets ending at NOW (12:00:30). A minute is
        // attributed to the bucket that holds its end.
        let bucket_of_minute = |m: i64| b.index_of((m + 1) * MINUTE_MS).unwrap();
        let pm = minute_of(prev_ts);
        let mut expected = vec![0u64; 5];
        expected[bucket_of_minute(pm)] += 4;
        expected[bucket_of_minute(pm - 1)] += 7;
        // pm-3 ends exactly… check it is still in the window before adding.
        if let Some(i) = b.index_of((pm - 3 + 1) * MINUTE_MS) {
            expected[i] += 3;
        }
        assert_eq!(series, expected);
        assert_eq!(series.iter().sum::<u64>(), 14);
    }

    #[test]
    fn native_points_older_than_the_window_are_ignored_and_empty_means_not_recording() {
        let native = NativeMetrics {
            count: 50,
            prev_count: 50,
            prev_ts_ms: NOW - 10 * MINUTE_MS,
            data: vec![9; 100],
        };
        let series = native.series(&Buckets::new(MetricsRange::M5, NOW));
        assert_eq!(
            series.iter().sum::<u64>(),
            0,
            "nothing in the last 5 minutes"
        );
        let day = native.series(&Buckets::new(MetricsRange::H24, NOW));
        assert_eq!(
            day.iter().sum::<u64>(),
            900,
            "all 100 minutes are within 24h"
        );

        let idle = NativeMetrics::default();
        assert!(!idle.is_recording());
        assert_eq!(
            idle.series(&Buckets::new(MetricsRange::H1, NOW)),
            vec![0; 12]
        );
    }

    fn done(id: &str, name: &str, finished_ago_ms: i64, took_ms: i64, waited_ms: i64) -> JobSample {
        let finished_on = NOW - finished_ago_ms;
        let processed_on = finished_on - took_ms;
        JobSample {
            job_id: id.into(),
            name: name.into(),
            timestamp: Some(processed_on - waited_ms),
            processed_on: Some(processed_on),
            finished_on: Some(finished_on),
            delay: None,
            failed_reason: None,
        }
    }

    // "usual" is the median per job type.
    #[test]
    fn usual_durations_are_medians_per_job_type() {
        let b = Buckets::new(MetricsRange::H1, NOW);
        let completed = [
            done("1", "resize", 60_000, 100, 0),
            done("2", "resize", 120_000, 900, 0),
            done("3", "resize", 180_000, 300, 0),
            done("4", "send", 60_000, 200, 0),
            done("5", "send", 70_000, 400, 0),
            done("old", "send", 3 * 3_600_000, 99_999, 0), // outside the window
        ];
        let m = build(QueueMetricsInput {
            queue_name: "emails",
            buckets: &b,
            native_completed: None,
            native_failed: None,
            estimated_completed: &[],
            estimated_failed: &[],
            completed_sample: &completed,
            failed_sample: &[],
        });
        let usual: Vec<(&str, u64, u32)> = m
            .usual_durations
            .iter()
            .map(|u| (u.name.as_str(), u.median_ms, u.samples))
            .collect();
        assert_eq!(usual, vec![("resize", 300, 3), ("send", 300, 2)]);
        assert!(m.usual_durations.iter().all(|u| u.queue_name == "emails"));
    }

    #[test]
    fn delayed_jobs_are_not_charged_their_own_delay() {
        let mut job = done("1", "send", 1_000, 200, 60_500);
        job.delay = Some(60_000);
        assert_eq!(job.queue_delay_ms(), Some(500));
        assert_eq!(job.processing_time_ms(), Some(200));
        // Garbage in → no figure, not a panic or a huge number.
        let broken = JobSample {
            processed_on: Some(10),
            finished_on: Some(5),
            ..Default::default()
        };
        assert_eq!(broken.processing_time_ms(), None);
    }

    #[test]
    fn estimated_counts_timing_slowest_and_failing_types() {
        let b = Buckets::new(MetricsRange::H1, NOW);
        let completed = [
            done("1", "resize", 60_000, 100, 10),
            done("2", "resize", 120_000, 900, 30),
            done("3", "send", 50 * 60_000, 300, 20),
            done("old", "send", 3 * 3_600_000, 99_999, 0), // outside the window
        ];
        let mut f1 = done("f1", "send-broken", 30_000, 5, 0);
        f1.failed_reason = Some("ETIMEDOUT".into());
        let mut f2 = done("f2", "send-broken", 90_000, 5, 0);
        f2.failed_reason = Some("older reason".into());
        let mut f3 = done("f3", "charge", 40_000, 5, 0);
        f3.failed_reason = Some("card declined".into());
        let failed = [f1, f2, f3];

        let mut est_completed = vec![0u64; 12];
        est_completed[11] = 2;
        est_completed[1] = 1;
        let mut est_failed = vec![0u64; 12];
        est_failed[11] = 3;

        let m = build(QueueMetricsInput {
            queue_name: "emails",
            buckets: &b,
            native_completed: None,
            native_failed: None,
            estimated_completed: &est_completed,
            estimated_failed: &est_failed,
            completed_sample: &completed,
            failed_sample: &failed,
        });

        assert_eq!((m.native_queues, m.total_queues), (0, 1), "estimated");
        assert_eq!((m.completed_total, m.failed_total), (3, 3));
        assert!((m.failure_rate - 0.5).abs() < 1e-9);
        assert!((m.throughput_per_minute - 3.0 / 60.0).abs() < 1e-9);
        assert_eq!(m.sampled_jobs, 3, "the out-of-window job is not sampled");
        assert_eq!(
            (
                m.min_processing_time_ms,
                m.avg_processing_time_ms,
                m.max_processing_time_ms
            ),
            (100, 433, 900)
        );
        assert_eq!((m.min_queue_delay_ms, m.max_queue_delay_ms), (10, 30));
        assert_eq!(m.processing_time_series_ms[11], 500, "avg of 100 and 900");
        assert_eq!(m.bucket_starts_ms.len(), 12);

        assert_eq!(
            m.slowest_jobs
                .iter()
                .map(|j| j.job_id.as_str())
                .collect::<Vec<_>>(),
            vec!["2", "3", "1"]
        );
        assert_eq!(m.failing_job_types.len(), 2);
        let top = &m.failing_job_types[0];
        assert_eq!((top.name.as_str(), top.count), ("send-broken", 2));
        assert_eq!(
            top.last_reason.as_deref(),
            Some("ETIMEDOUT"),
            "the most recent reason"
        );
    }

    #[test]
    fn native_counts_win_over_the_estimate_when_the_queue_records_them() {
        let b = Buckets::new(MetricsRange::M5, NOW);
        let native = NativeMetrics {
            count: 1_000,
            prev_count: 990,
            prev_ts_ms: NOW - 1_000,
            data: vec![20, 20, 20, 20],
        };
        let m = build(QueueMetricsInput {
            queue_name: "images",
            buckets: &b,
            native_completed: Some(&native),
            native_failed: Some(&NativeMetrics::default()),
            // removeOnComplete left almost nothing in Redis:
            estimated_completed: &[0, 0, 0, 0, 1],
            estimated_failed: &[0; 5],
            completed_sample: &[],
            failed_sample: &[],
        });
        assert_eq!(m.native_queues, 1);
        assert_eq!(
            m.completed_total, 90,
            "10 running + 4×20, not the estimate's 1"
        );
        assert_eq!(m.failed_total, 0);
    }

    #[test]
    fn merging_queues_sums_counts_and_weights_averages() {
        let b = Buckets::new(MetricsRange::H1, NOW);
        let q = |name: &str, jobs: &[JobSample], est: u64, native: bool| {
            let mut series = vec![0u64; 12];
            series[11] = est;
            let recording = NativeMetrics {
                count: est.max(1),
                prev_count: 0,
                prev_ts_ms: NOW - 1_000,
                data: vec![],
            };
            build(QueueMetricsInput {
                queue_name: name,
                buckets: &b,
                native_completed: native.then_some(&recording),
                native_failed: None,
                estimated_completed: &series,
                estimated_failed: &[],
                completed_sample: jobs,
                failed_sample: &[],
            })
        };
        let fast = q(
            "fast",
            &[
                done("a", "x", 1_000, 100, 0),
                done("b", "x", 2_000, 100, 0),
                done("c", "x", 3_000, 100, 0),
            ],
            3,
            true,
        );
        let slow = q("slow", &[done("z", "y", 1_000, 900, 0)], 1, false);

        let merged = merge(MetricsRange::H1, NOW, &[fast, slow]);
        assert_eq!(merged.completed_total, 4);
        assert_eq!((merged.native_queues, merged.total_queues), (1, 2));
        assert_eq!(merged.avg_processing_time_ms, 300, "(3×100 + 1×900) / 4");
        assert_eq!(
            (merged.min_processing_time_ms, merged.max_processing_time_ms),
            (100, 900)
        );
        assert_eq!(merged.slowest_jobs[0].queue_name, "slow");
        assert_eq!(merged.slowest_jobs.len(), 4);

        let empty = merge(MetricsRange::M5, NOW, &[]);
        assert_eq!(empty.total_queues, 0);
        assert_eq!(empty.completed_series, vec![0; 5]);
    }

    // The connection's throughput, queue by queue.
    #[test]
    fn throughput_sums_recording_queues_and_flags_the_others() {
        let q = |name: &str| crate::core::bullmq_schema::domain::QueueRef {
            prefix: "bull".into(),
            name: name.into(),
        };
        // `prevTS` on the minute boundary 2 minutes before NOW: the list head
        // is the minute before that one.
        let prev_ts = (NOW.div_euclid(MINUTE_MS) - 2) * MINUTE_MS;
        let recording = NativeMetrics {
            count: 30,
            prev_count: 25,
            prev_ts_ms: prev_ts,
            data: vec![4, 0, 6],
        };
        let failing = NativeMetrics {
            count: 2,
            prev_count: 2,
            prev_ts_ms: prev_ts,
            data: vec![1, 1],
        };
        let t = throughput(
            MetricsRange::H1,
            NOW,
            vec![
                (q("emails"), recording.clone(), failing),
                (
                    q("silent"),
                    NativeMetrics::default(),
                    NativeMetrics::default(),
                ),
            ],
        );
        assert_eq!(t.bucket_starts_ms.len(), 12);
        assert_eq!(t.metrics_off_queues, 1);
        let silent = &t.queues[1];
        assert!(silent.metrics_off);
        assert!(
            silent.completed.is_empty(),
            "metrics off: no series, not zeros"
        );
        let emails = &t.queues[0];
        assert!(!emails.metrics_off);
        assert_eq!(
            emails.completed.iter().sum::<u64>(),
            15,
            "5 running + 4 + 6"
        );
        assert_eq!(emails.failed.iter().sum::<u64>(), 2);
        assert_eq!(
            t.completed, emails.completed,
            "the total sums recording queues"
        );
        assert_eq!(
            t.completed,
            recording.series(&Buckets::new(MetricsRange::H1, NOW))
        );
    }
}
