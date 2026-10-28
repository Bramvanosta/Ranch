//! Job schedules: validating and normalising a scheduler as the user (or an
//! agent) describes it, and computing its next runs.
//!
//! A BullMQ job scheduler's pending job is created by the *worker* that
//! finishes the previous one, computing the following run itself
//! (cron-parser, in JS). So Ranch only ever computes the **first** run of a
//! cron pattern, and must only accept patterns that BullMQ's parser reads
//! the same way.

use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;
use croner::parser::{CronParser, Seconds};

use crate::jobs::{minify_json, MAX_DATA_BYTES};
use crate::queues::QueueId;

pub const MAX_ID_LEN: usize = 128;
/// A typo guard: one run per second is already a lot for a scheduler.
pub const MIN_EVERY_MS: u64 = 1_000;
/// ~10 years.
pub const MAX_EVERY_MS: u64 = 10 * 365 * 24 * 60 * 60 * 1000;
pub const PREVIEW_RUNS: usize = 5;

/// When a scheduler fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schedule {
    /// A cron expression, 5 fields (minute first) or 6 (second first), read
    /// in `tz` (IANA name), UTC when `None`.
    Pattern { pattern: String, tz: Option<String> },
    /// A fixed interval.
    Every { every_ms: u64 },
}

/// A scheduler as the user (or an agent) describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerSpec {
    /// The scheduler id: BullMQ's key. Saving an existing id edits it.
    pub id: String,
    /// Name of the jobs it produces.
    pub job_name: String,
    pub schedule: Schedule,
    /// JSON template: the `data` of every job produced. `{}` when empty.
    pub data: String,
}

/// Validated, normalized: ready for `upsert_scheduler.lua`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedScheduler {
    pub id: String,
    pub job_name: String,
    pub pattern: Option<String>,
    pub tz: Option<String>,
    pub every_ms: Option<u64>,
    /// Minified JSON template.
    pub data: String,
    /// First run of a `pattern` (ms). `None` for `every`: the script derives
    /// it from the interval and the scheduler's previous state, as BullMQ does.
    pub first_run_ms: Option<i64>,
}

/// The cron dialect both BullMQ's parser (cron-parser) and Ranch's (croner)
/// read identically: numbers, `*`, lists, ranges, steps, and month / weekday
/// names. `L`, `W`, `#`, `?` and `@aliases` exist in both but with
/// differences; a pattern Ranch previews one way and the worker runs another
/// way is worse than a refused pattern.
fn check_dialect(pattern: &str) -> Result<(), String> {
    let fields: Vec<&str> = pattern.split_whitespace().collect();
    if fields.len() != 5 && fields.len() != 6 {
        return Err(format!(
            "A cron pattern has 5 fields (minute hour day month weekday) or 6 with seconds first; this one has {}",
            fields.len()
        ));
    }
    for field in &fields {
        let mut letters = 0;
        for c in field.chars() {
            match c {
                '0'..='9' | '*' | ',' | '-' | '/' => {}
                'a'..='z' | 'A'..='Z' => letters += 1,
                other => {
                    return Err(format!(
                        "'{other}' is not supported in a cron pattern here; use numbers, * , - / and month or weekday names"
                    ))
                }
            }
        }
        // `L`, `W` … are single letters; names are three (JAN, mon).
        if letters > 0 && letters % 3 != 0 {
            return Err(format!(
                "'{field}' is not supported: month and weekday names have three letters (JAN, MON)"
            ));
        }
    }
    Ok(())
}

fn parse_tz(tz: Option<&str>) -> Result<Tz, String> {
    match tz.map(str::trim).filter(|t| !t.is_empty()) {
        None => Ok(Tz::UTC),
        Some(name) => name.parse::<Tz>().map_err(|_| {
            format!("Unknown timezone \"{name}\"; use an IANA name such as Europe/Paris")
        }),
    }
}

fn parse_pattern(pattern: &str) -> Result<croner::Cron, String> {
    check_dialect(pattern)?;
    CronParser::builder()
        .seconds(Seconds::Optional)
        .build()
        .parse(pattern)
        .map_err(|e| format!("Invalid cron pattern: {e}"))
}

impl Schedule {
    /// The next `count` runs strictly after `after`.
    pub fn next_runs(&self, after: DateTime<Utc>, count: usize) -> Result<Vec<i64>, String> {
        match self {
            Schedule::Pattern { pattern, tz } => {
                let cron = parse_pattern(pattern.trim())?;
                let zone = parse_tz(tz.as_deref())?;
                let start = zone.from_utc_datetime(&after.naive_utc());
                let runs: Vec<i64> = cron
                    .iter_after(start)
                    .take(count)
                    .map(|t| t.timestamp_millis())
                    .collect();
                if runs.is_empty() {
                    return Err("This pattern never fires".into());
                }
                Ok(runs)
            }
            Schedule::Every { every_ms } => {
                check_every(*every_ms)?;
                let every =
                    i64::try_from(*every_ms).map_err(|_| "Interval too large".to_string())?;
                // A new `every` scheduler fires now, then every interval.
                let now = after.timestamp_millis();
                Ok((0..count as i64).map(|i| now + i * every).collect())
            }
        }
    }
}

/// Most runs drawn in a window: a dot per run stops telling
/// anything long before this.
pub const MAX_WINDOW_RUNS: usize = 500;

impl Schedule {
    /// The runs from `from` (exclusive) up to `until`
    /// (inclusive), at most `max`. For `every`, runs are anchored on
    /// `first` (the scheduler's next run), so the dots line up with BullMQ.
    pub fn runs_within(
        &self,
        from: DateTime<Utc>,
        until: DateTime<Utc>,
        first: Option<i64>,
        max: usize,
    ) -> Result<Vec<i64>, String> {
        let max = max.min(MAX_WINDOW_RUNS);
        let end = until.timestamp_millis();
        match self {
            Schedule::Pattern { pattern, tz } => {
                let cron = parse_pattern(pattern.trim())?;
                let zone = parse_tz(tz.as_deref())?;
                let start = zone.from_utc_datetime(&from.naive_utc());
                Ok(cron
                    .iter_after(start)
                    .map(|t| t.timestamp_millis())
                    .take_while(|t| *t <= end)
                    .take(max)
                    .collect())
            }
            Schedule::Every { every_ms } => {
                check_every(*every_ms)?;
                let every =
                    i64::try_from(*every_ms).map_err(|_| "Interval too large".to_string())?;
                let start = from.timestamp_millis();
                // The first run after `from` on the scheduler's own grid.
                let mut t = first.unwrap_or(start + every);
                if t <= start {
                    t += ((start - t) / every + 1) * every;
                }
                let mut runs = Vec::new();
                while t <= end && runs.len() < max {
                    runs.push(t);
                    t += every;
                }
                Ok(runs)
            }
        }
    }
}

fn check_every(every_ms: u64) -> Result<(), String> {
    if every_ms < MIN_EVERY_MS {
        return Err(format!("The interval must be at least {MIN_EVERY_MS} ms"));
    }
    if every_ms > MAX_EVERY_MS {
        return Err("The interval is longer than 10 years".into());
    }
    Ok(())
}

impl SchedulerSpec {
    pub fn prepare(&self, now: DateTime<Utc>) -> Result<PreparedScheduler, String> {
        let id = self.id.trim();
        if id.is_empty() {
            return Err("The scheduler needs an id".into());
        }
        if id.chars().count() > MAX_ID_LEN {
            return Err(format!("The id is longer than {MAX_ID_LEN} characters"));
        }
        // BullMQ builds `repeat:{id}:{millis}` job ids and splits them on ':'.
        if id.contains(':') {
            return Err("The id cannot contain ':'".into());
        }
        if id.chars().any(char::is_whitespace) {
            return Err("The id cannot contain spaces".into());
        }
        let job_name = self.job_name.trim();
        if job_name.is_empty() {
            return Err("Job name is required".into());
        }
        if job_name.contains(':') {
            return Err("Job name cannot contain ':'".into());
        }
        let data = self.data.trim();
        let data = if data.is_empty() { "{}" } else { data };
        if data.len() > MAX_DATA_BYTES {
            return Err("The data template is larger than 1024 KB".into());
        }
        serde_json::from_str::<serde::de::IgnoredAny>(data)
            .map_err(|e| format!("The data template is not valid JSON: {e}"))?;
        let data = minify_json(data);

        match &self.schedule {
            Schedule::Pattern { pattern, tz } => {
                let pattern = pattern.split_whitespace().collect::<Vec<_>>().join(" ");
                let tz = tz
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string);
                let normalized = Schedule::Pattern {
                    pattern: pattern.clone(),
                    tz: tz.clone(),
                };
                let first = normalized.next_runs(now, 1)?[0];
                Ok(PreparedScheduler {
                    id: id.to_string(),
                    job_name: job_name.to_string(),
                    pattern: Some(pattern),
                    tz,
                    every_ms: None,
                    data,
                    first_run_ms: Some(first),
                })
            }
            Schedule::Every { every_ms } => {
                check_every(*every_ms)?;
                Ok(PreparedScheduler {
                    id: id.to_string(),
                    job_name: job_name.to_string(),
                    pattern: None,
                    tz: None,
                    every_ms: Some(*every_ms),
                    data,
                    first_run_ms: None,
                })
            }
        }
    }
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
#[derive(Debug, Clone, PartialEq)]
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

/// What saving a scheduler produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerSaved {
    pub scheduler_id: String,
    /// When its pending job runs (ms).
    pub next_run_ms: i64,
    /// Id of that pending job: `repeat:{id}:{next run}`.
    pub job_id: String,
}

/// One scheduler of a queue, as an operation names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerTarget {
    pub queue_id: QueueId,
    /// The scheduler's key (`CronJob::key`).
    pub scheduler_key: String,
}

/// Where a scheduler stands after a pause or a resume. `already_in_state`
/// is true when it was there before the call, which then changed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerActionResult {
    pub paused: bool,
    pub already_in_state: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn pattern(p: &str, tz: Option<&str>) -> Schedule {
        Schedule::Pattern {
            pattern: p.into(),
            tz: tz.map(Into::into),
        }
    }

    fn iso(ms: i64) -> String {
        Utc.timestamp_millis_opt(ms).unwrap().to_rfc3339()
    }

    #[test]
    fn next_runs_of_a_five_field_pattern_in_utc() {
        let runs = pattern("0 9 * * *", None)
            .next_runs(at("2026-09-18T10:00:00Z"), 3)
            .unwrap();
        assert_eq!(
            runs.iter().map(|r| iso(*r)).collect::<Vec<_>>(),
            [
                "2026-09-19T09:00:00+00:00",
                "2026-09-20T09:00:00+00:00",
                "2026-09-21T09:00:00+00:00"
            ]
        );
    }

    #[test]
    fn a_timezone_moves_the_run_and_follows_daylight_saving() {
        // 09:00 in Paris = 07:00 UTC in summer, 08:00 UTC in winter.
        let paris = pattern("0 9 * * *", Some("Europe/Paris"));
        let summer = paris.next_runs(at("2026-09-18T10:00:00Z"), 1).unwrap()[0];
        assert_eq!(iso(summer), "2026-09-19T07:00:00+00:00");
        let winter = paris.next_runs(at("2026-12-01T10:00:00Z"), 1).unwrap()[0];
        assert_eq!(iso(winter), "2026-12-02T08:00:00+00:00");
    }

    #[test]
    fn six_fields_have_seconds_first_and_names_are_accepted() {
        let every_10s = pattern("*/10 * * * * *", None)
            .next_runs(at("2026-09-18T10:00:03Z"), 2)
            .unwrap();
        assert_eq!(iso(every_10s[0]), "2026-09-18T10:00:10+00:00");
        assert_eq!(iso(every_10s[1]), "2026-09-18T10:00:20+00:00");
        let monday = pattern("30 8 * JAN-DEC MON", None)
            .next_runs(at("2026-09-18T10:00:00Z"), 1)
            .unwrap()[0];
        assert_eq!(
            iso(monday),
            "2026-09-21T08:30:00+00:00",
            "2026-09-21 is a Monday"
        );
    }

    #[test]
    fn the_run_is_strictly_after_now() {
        let runs = pattern("0 10 * * *", None)
            .next_runs(at("2026-09-18T10:00:00Z"), 1)
            .unwrap();
        assert_eq!(iso(runs[0]), "2026-09-19T10:00:00+00:00");
    }

    #[test]
    fn refuses_what_bullmq_would_read_differently_or_not_at_all() {
        let bad = [
            ("0 9 * *", "5 fields"),
            ("0 9 * * * * *", "5 fields"),
            ("0 9 L * *", "not supported"),
            ("0 9 15W * *", "not supported"),
            ("0 9 * * 1#2", "not supported"),
            ("0 9 ? * *", "not supported"),
            ("@daily", "5 fields"),
            ("0 25 * * *", "Invalid cron pattern"),
            ("61 * * * *", "Invalid cron pattern"),
        ];
        for (p, expected) in bad {
            let err = pattern(p, None)
                .next_runs(at("2026-09-18T10:00:00Z"), 1)
                .expect_err(p);
            assert!(err.contains(expected), "{p}: {err}");
        }
        let err = pattern("0 9 * * *", Some("Mars/Olympus"))
            .next_runs(at("2026-09-18T10:00:00Z"), 1)
            .expect_err("tz");
        assert!(err.contains("Unknown timezone"), "{err}");
    }

    #[test]
    fn every_fires_now_then_at_each_interval() {
        let now = at("2026-09-18T10:00:00Z");
        let runs = Schedule::Every { every_ms: 60_000 }
            .next_runs(now, 3)
            .unwrap();
        let t = now.timestamp_millis();
        assert_eq!(runs, [t, t + 60_000, t + 120_000]);
        assert!(Schedule::Every { every_ms: 999 }.next_runs(now, 1).is_err());
        assert!(Schedule::Every {
            every_ms: MAX_EVERY_MS + 1
        }
        .next_runs(now, 1)
        .is_err());
    }

    fn spec(schedule: Schedule) -> SchedulerSpec {
        SchedulerSpec {
            id: " nightly-report ".into(),
            job_name: " build-report ".into(),
            schedule,
            data: "{ \"kind\": \"summary\" }".into(),
        }
    }

    #[test]
    fn prepare_normalizes_and_computes_the_first_run_of_a_pattern() {
        let now = at("2026-09-18T10:00:00Z");
        let p = spec(pattern("  0   9 * * * ", Some(" Europe/Paris ")))
            .prepare(now)
            .unwrap();
        assert_eq!(p.id, "nightly-report");
        assert_eq!(p.job_name, "build-report");
        assert_eq!(p.pattern.as_deref(), Some("0 9 * * *"));
        assert_eq!(p.tz.as_deref(), Some("Europe/Paris"));
        assert_eq!(p.every_ms, None);
        assert_eq!(p.data, r#"{"kind":"summary"}"#);
        assert_eq!(iso(p.first_run_ms.unwrap()), "2026-09-19T07:00:00+00:00");

        let e = spec(Schedule::Every { every_ms: 5_000 })
            .prepare(now)
            .unwrap();
        assert_eq!(
            (e.pattern, e.every_ms, e.first_run_ms),
            (None, Some(5_000), None)
        );

        let blank_tz = spec(pattern("0 9 * * *", Some("  "))).prepare(now).unwrap();
        assert_eq!(blank_tz.tz, None);
        let empty_data = SchedulerSpec {
            data: "  ".into(),
            ..spec(Schedule::Every { every_ms: 5_000 })
        };
        assert_eq!(empty_data.prepare(now).unwrap().data, "{}");
    }

    #[test]
    fn prepare_refuses_ids_and_names_bullmq_cannot_key() {
        let now = at("2026-09-18T10:00:00Z");
        let base = spec(Schedule::Every { every_ms: 5_000 });
        let cases: Vec<(SchedulerSpec, &str)> = vec![
            (
                SchedulerSpec {
                    id: "  ".into(),
                    ..base.clone()
                },
                "needs an id",
            ),
            (
                SchedulerSpec {
                    id: "a:b".into(),
                    ..base.clone()
                },
                "cannot contain ':'",
            ),
            (
                SchedulerSpec {
                    id: "a b".into(),
                    ..base.clone()
                },
                "spaces",
            ),
            (
                SchedulerSpec {
                    id: "x".repeat(129),
                    ..base.clone()
                },
                "longer than",
            ),
            (
                SchedulerSpec {
                    job_name: "".into(),
                    ..base.clone()
                },
                "Job name is required",
            ),
            (
                SchedulerSpec {
                    data: "{oops".into(),
                    ..base.clone()
                },
                "not valid JSON",
            ),
        ];
        for (input, expected) in cases {
            let err = input.prepare(now).expect_err(expected);
            assert!(err.contains(expected), "{err}");
        }
    }

    // The runs of a window, for the "Next 24 hours" strip.
    #[test]
    fn runs_within_a_window() {
        let from = Utc.with_ymd_and_hms(2026, 10, 26, 22, 0, 0).unwrap();
        let until = from + chrono::Duration::hours(24);
        let hourly = Schedule::Pattern {
            pattern: "0 * * * *".into(),
            tz: None,
        };
        let runs = hourly.runs_within(from, until, None, 100).unwrap();
        assert_eq!(runs.len(), 24, "every hour on the hour, 23:00 to 22:00");
        assert_eq!(
            runs[0],
            (from + chrono::Duration::hours(1)).timestamp_millis()
        );
        assert!(runs.windows(2).all(|w| w[1] - w[0] == 3_600_000));
        // Capped.
        let minutely = Schedule::Pattern {
            pattern: "* * * * *".into(),
            tz: None,
        };
        assert_eq!(
            minutely
                .runs_within(from, until, None, 10_000)
                .unwrap()
                .len(),
            MAX_WINDOW_RUNS
        );

        // `every` keeps the scheduler's grid, from its next run.
        let every = Schedule::Every {
            every_ms: 6 * 3_600_000,
        };
        let first = from.timestamp_millis() - 3_600_000; // already passed
        let runs = every.runs_within(from, until, Some(first), 100).unwrap();
        assert_eq!(runs.len(), 4);
        assert_eq!(runs[0], first + 6 * 3_600_000);
    }
}
