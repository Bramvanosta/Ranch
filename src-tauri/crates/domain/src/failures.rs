//! Failed jobs across the queues of a connection.
//!
//! Pure logic: merge the queues' `failed` sets by failure time, filter, page,
//! summarise by job type, and group by cause. The actor
//! fetches; this decides. Unit-tested without Redis.
//!
//! The scan is bounded (the newest [`PER_QUEUE_CAP`] failed jobs of each
//! queue, [`TOTAL_CAP`] overall), so a connection with a million failed jobs
//! costs the same as one with a few hundred. The view is about *what is
//! failing now*; deep archaeology stays in the per-queue list.

use super::jobs::{Job, JobRef};
use super::metrics::{FailingJobType, TOP_N};
use super::queues::QueueId;

/// Newest failed jobs looked at per queue.
pub const PER_QUEUE_CAP: usize = 500;
/// …and across the whole connection.
pub const TOTAL_CAP: usize = 3000;
pub const MAX_PAGE_SIZE: u32 = 250;

/// One failed job as the scan sees it: enough to order, filter and group.
#[derive(Debug, Clone, PartialEq)]
pub struct FailedCandidate {
    pub queue_id: QueueId,
    pub queue_name: String,
    pub job_id: String,
    /// Score in the `failed` sorted set = `finishedOn` (ms).
    pub failed_at: i64,
    pub name: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FailedFilter {
    /// Exact queue name.
    pub queue: Option<String>,
    /// Exact job name (what the "by job type" summary links to).
    pub name: Option<String>,
    /// Case-insensitive substring of the failure reason.
    pub reason: Option<String>,
    /// Exact cause signature ([`failure_signature`]).
    pub signature: Option<String>,
}

impl FailedFilter {
    pub fn is_empty(&self) -> bool {
        self.queue.is_none()
            && self.name.is_none()
            && self.reason.is_none()
            && self.signature.is_none()
    }

    fn non_empty(v: &Option<String>) -> Option<&str> {
        v.as_deref().map(str::trim).filter(|s| !s.is_empty())
    }

    pub fn matches(&self, c: &FailedCandidate) -> bool {
        if let Some(q) = Self::non_empty(&self.queue) {
            if c.queue_name != q {
                return false;
            }
        }
        if let Some(n) = Self::non_empty(&self.name) {
            if c.name != n {
                return false;
            }
        }
        if let Some(r) = Self::non_empty(&self.reason) {
            let needle = r.to_lowercase();
            if !c
                .reason
                .as_deref()
                .is_some_and(|reason| reason.to_lowercase().contains(&needle))
            {
                return false;
            }
        }
        // Not trimmed: the empty signature is the cause "no reason recorded".
        if let Some(sig) = &self.signature {
            if &failure_signature(c.reason.as_deref().unwrap_or("")) != sig {
                return false;
            }
        }
        true
    }
}

/// A page of the connection's failed jobs, newest failure first, and
/// what they must match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FailedJobsQuery {
    pub offset: u32,
    /// Page size, clamped to 1–[`MAX_PAGE_SIZE`].
    pub limit: u32,
    pub filter: FailedFilter,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FailedQueueCount {
    pub queue_id: QueueId,
    pub queue_name: String,
    /// `ZCARD` of the queue's failed set, exact.
    pub failed: u32,
}

/// One page of the cross-queue failed view.
#[derive(Debug, Clone, Default, PartialEq)]
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

/// Merge per-queue candidates, newest first, bounded by [`TOTAL_CAP`].
pub fn merge_candidates(mut all: Vec<FailedCandidate>) -> Vec<FailedCandidate> {
    all.sort_by(|a, b| {
        b.failed_at
            .cmp(&a.failed_at)
            .then_with(|| a.queue_name.cmp(&b.queue_name))
            .then_with(|| b.job_id.cmp(&a.job_id))
    });
    all.truncate(TOTAL_CAP);
    all
}

/// Most failing job types among `candidates` (restricted to `queue` when set).
pub fn summarise_types(candidates: &[FailedCandidate], queue: Option<&str>) -> Vec<FailingJobType> {
    let mut types: Vec<FailingJobType> = Vec::new();
    for c in candidates {
        if queue.is_some_and(|q| c.queue_name != q) {
            continue;
        }
        match types
            .iter_mut()
            .find(|t| t.name == c.name && t.queue_name == c.queue_name)
        {
            Some(t) => {
                t.count += 1;
                if c.failed_at > t.last_failed_at {
                    t.last_failed_at = c.failed_at;
                    t.last_reason = c.reason.clone();
                }
            }
            None => types.push(FailingJobType {
                queue_id: c.queue_id.clone(),
                queue_name: c.queue_name.clone(),
                name: c.name.clone(),
                count: 1,
                last_failed_at: c.failed_at,
                last_reason: c.reason.clone(),
            }),
        }
    }
    types.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then(b.last_failed_at.cmp(&a.last_failed_at))
    });
    types.truncate(TOP_N);
    types
}

/// The slice of `candidates` a page shows, plus how many match in total.
pub fn page_of<'a>(
    candidates: &'a [FailedCandidate],
    filter: &FailedFilter,
    offset: u32,
    limit: u32,
) -> (Vec<&'a FailedCandidate>, u32) {
    let matching: Vec<&FailedCandidate> = candidates.iter().filter(|c| filter.matches(c)).collect();
    let total = matching.len() as u32;
    let limit = limit.clamp(1, MAX_PAGE_SIZE) as usize;
    let page = matching
        .into_iter()
        .skip(offset as usize)
        .take(limit)
        .collect();
    (page, total)
}

// ---------------------------------------------------------------------------
// Failures grouped by cause
// ---------------------------------------------------------------------------

/// Longest signature kept; a reason longer than this is one cause anyway.
const MAX_SIGNATURE_CHARS: usize = 400;
/// Hourly buckets of [`FailureCause::trend`].
pub const TREND_HOURS: usize = 24;
const HOUR_MS: i64 = 3_600_000;
/// Job types named per cause.
const MAX_JOB_NAMES: usize = 5;

/// One cause: the failures whose reasons only differ by the values in them.
#[derive(Debug, Clone, PartialEq)]
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

/// The failed jobs of a queue (or of a connection), grouped by cause.
#[derive(Debug, Clone, Default, PartialEq)]
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

/// What two failures share when they have "the same cause": the first line
/// of the reason with its values normalised (URLs, quoted values,
/// timestamps, UUIDs, IP addresses, hex and numbers) and its spaces
/// collapsed.
///
/// `connect ETIMEDOUT 10.0.4.12:5432` → `connect ETIMEDOUT <ip>`;
/// `Job 1232 failed after 3 attempts` → `Job <n> failed after <n> attempts`.
pub fn failure_signature(reason: &str) -> String {
    let line = reason
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        // URL: scheme "://" then everything up to a space or a closing mark.
        if ch.is_ascii_alphabetic() && word_start(&chars, i) {
            if let Some(end) = url_end(&chars, i) {
                out.push_str("<url>");
                i = end;
                continue;
            }
        }
        // Quoted value: "…", `…`, and '…' when the ' is not an apostrophe.
        if (ch == '"' || ch == '`' || (ch == '\'' && word_start(&chars, i)))
            && chars[i + 1..].contains(&ch)
        {
            if let Some(close) = chars[i + 1..].iter().position(|&c| c == ch) {
                let end = i + 1 + close + 1;
                if ch != '\'' || end >= chars.len() || !chars[end].is_alphanumeric() {
                    out.push_str("<str>");
                    i = end;
                    continue;
                }
            }
        }
        if is_token_char(ch) {
            let start = i;
            while i < chars.len() && is_token_char(chars[i]) {
                i += 1;
            }
            let token: String = chars[start..i].iter().collect();
            // Sentence punctuation is not part of the value.
            let core = token.trim_end_matches(['.', ':', ',', '-']);
            out.push_str(&normalise_token(core));
            out.push_str(&token[core.len()..]);
            continue;
        }
        if ch.is_whitespace() {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(ch);
        }
        i += 1;
    }
    let trimmed = out.trim();
    trimmed.chars().take(MAX_SIGNATURE_CHARS).collect()
}

fn is_token_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')
}

fn word_start(chars: &[char], i: usize) -> bool {
    i == 0 || !chars[i - 1].is_alphanumeric()
}

/// End (exclusive) of a URL starting at `i`, if one does.
fn url_end(chars: &[char], i: usize) -> Option<usize> {
    let mut j = i;
    while j < chars.len()
        && (chars[j].is_ascii_alphanumeric() || matches!(chars[j], '+' | '.' | '-'))
    {
        j += 1;
    }
    if j + 2 < chars.len() && chars[j] == ':' && chars[j + 1] == '/' && chars[j + 2] == '/' {
        let mut end = j + 3;
        while end < chars.len()
            && !chars[end].is_whitespace()
            && !matches!(chars[end], '"' | '\'' | '`' | ')' | ']' | '>' | ',')
        {
            end += 1;
        }
        return Some(end);
    }
    None
}

fn normalise_token(token: &str) -> String {
    if token.is_empty() || !token.chars().any(|c| c.is_ascii_digit()) {
        return token.to_string();
    }
    if is_timestamp(token) {
        return "<time>".into();
    }
    if is_uuid(token) {
        return "<id>".into();
    }
    if is_ip(token) {
        return "<ip>".into();
    }
    let lower = token.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix("0x") {
        if !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return "<hex>".into();
        }
    }
    // A Mongo ObjectId, a hash, a hex-encoded id.
    if token.len() >= 12
        && token.chars().all(|c| c.is_ascii_hexdigit())
        && token.chars().any(|c| c.is_ascii_alphabetic())
    {
        return "<hex>".into();
    }
    if token
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | ':' | '-' | '_'))
    {
        return "<n>".into();
    }
    // An identifier with a number in it ("job_12345", "worker-17"): its runs
    // of two digits or more are values; "utf8" or "v6" are words.
    let mut out = String::with_capacity(token.len());
    let chars: Vec<char> = token.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if i - start >= 2 {
                out.push_str("<n>");
            } else {
                out.extend(&chars[start..i]);
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// `2026-10-26`, `2026-10-26T18:31:12.000Z`, `18:31:12`.
fn is_timestamp(t: &str) -> bool {
    let b = t.as_bytes();
    let date = b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit);
    let time = b.len() >= 8
        && b[..2].iter().all(u8::is_ascii_digit)
        && b[2] == b':'
        && b[3..5].iter().all(u8::is_ascii_digit)
        && b[5] == b':'
        && b[6..8].iter().all(u8::is_ascii_digit);
    date || time
}

fn is_uuid(t: &str) -> bool {
    let parts: Vec<&str> = t.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(p, n)| p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// `10.0.4.12` or `10.0.4.12:5432`.
fn is_ip(t: &str) -> bool {
    let (addr, port) = match t.split_once(':') {
        Some((a, p)) => (a, Some(p)),
        None => (t, None),
    };
    let octets: Vec<&str> = addr.split('.').collect();
    octets.len() == 4
        && octets
            .iter()
            .all(|o| !o.is_empty() && o.len() <= 3 && o.chars().all(|c| c.is_ascii_digit()))
        && port.is_none_or(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Group `candidates` (newest first, as [`merge_candidates`] leaves them) by
/// cause, restricted to `queue` when set; most frequent cause first.
pub fn group_by_cause(
    candidates: &[FailedCandidate],
    queue: Option<&str>,
    now_ms: i64,
) -> Vec<FailureCause> {
    struct Acc {
        cause: FailureCause,
        names: Vec<(String, u32)>,
        queues: Vec<(String, u32)>,
    }
    fn bump(list: &mut Vec<(String, u32)>, key: &str) {
        match list.iter_mut().find(|(k, _)| k == key) {
            Some((_, n)) => *n += 1,
            None => list.push((key.to_string(), 1)),
        }
    }
    fn ranked(mut list: Vec<(String, u32)>, max: usize) -> Vec<String> {
        list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        list.into_iter().take(max).map(|(k, _)| k).collect()
    }

    let trend_start = now_ms - TREND_HOURS as i64 * HOUR_MS;
    let mut groups: Vec<Acc> = Vec::new();
    for c in candidates {
        if queue.is_some_and(|q| c.queue_name != q) {
            continue;
        }
        let reason = c.reason.as_deref().unwrap_or("");
        let signature = failure_signature(reason);
        let idx = match groups.iter().position(|g| g.cause.signature == signature) {
            Some(idx) => idx,
            None => {
                groups.push(Acc {
                    cause: FailureCause {
                        signature,
                        sample: reason.to_string(),
                        count: 0,
                        first_at: c.failed_at,
                        last_at: c.failed_at,
                        job_names: Vec::new(),
                        queue_names: Vec::new(),
                        trend: vec![0; TREND_HOURS],
                        jobs: Vec::new(),
                    },
                    names: Vec::new(),
                    queues: Vec::new(),
                });
                groups.len() - 1
            }
        };
        let g = &mut groups[idx];
        g.cause.count += 1;
        if c.failed_at > g.cause.last_at {
            g.cause.last_at = c.failed_at;
            g.cause.sample = reason.to_string();
        }
        g.cause.first_at = g.cause.first_at.min(c.failed_at);
        if c.failed_at >= trend_start && c.failed_at < now_ms {
            let bucket = ((c.failed_at - trend_start) / HOUR_MS) as usize;
            g.cause.trend[bucket.min(TREND_HOURS - 1)] += 1;
        } else if c.failed_at >= now_ms {
            // A clock a little ahead of ours: the latest hour.
            g.cause.trend[TREND_HOURS - 1] += 1;
        }
        bump(&mut g.names, &c.name);
        bump(&mut g.queues, &c.queue_name);
        g.cause.jobs.push(JobRef {
            queue_id: c.queue_id.clone(),
            queue_name: c.queue_name.clone(),
            id: c.job_id.clone(),
        });
    }

    let mut causes: Vec<FailureCause> = groups
        .into_iter()
        .map(|g| {
            let mut cause = g.cause;
            cause.job_names = ranked(g.names, MAX_JOB_NAMES);
            cause.queue_names = ranked(g.queues, usize::MAX);
            cause
        })
        .collect();
    causes.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then(b.last_at.cmp(&a.last_at))
            .then(a.signature.cmp(&b.signature))
    });
    causes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(queue: &str, id: &str, at: i64, name: &str, reason: &str) -> FailedCandidate {
        FailedCandidate {
            queue_id: QueueId::new(format!("bull:{queue}")),
            queue_name: queue.into(),
            job_id: id.into(),
            failed_at: at,
            name: name.into(),
            reason: (!reason.is_empty()).then(|| reason.to_string()),
        }
    }

    fn sample() -> Vec<FailedCandidate> {
        merge_candidates(vec![
            c(
                "emails",
                "1",
                100,
                "send-email",
                "connect ETIMEDOUT 10.0.0.42:587",
            ),
            c("emails", "2", 300, "send-email", "RateLimitError"),
            c(
                "reports",
                "9",
                200,
                "daily-report",
                "Cannot read property 'tenantId'",
            ),
            c(
                "emails",
                "3",
                400,
                "send-reminder",
                "connect ETIMEDOUT 10.0.0.42:587",
            ),
            c("images", "7", 250, "resize", ""),
        ])
    }

    #[test]
    fn merged_newest_first_across_queues() {
        let ids: Vec<String> = sample().iter().map(|c| c.job_id.clone()).collect();
        assert_eq!(ids, vec!["3", "2", "7", "9", "1"]);
    }

    #[test]
    fn the_scan_is_bounded() {
        let many: Vec<FailedCandidate> = (0..(TOTAL_CAP as i64 + 50))
            .map(|i| c("emails", &i.to_string(), i, "x", ""))
            .collect();
        let merged = merge_candidates(many);
        assert_eq!(merged.len(), TOTAL_CAP);
        assert_eq!(
            merged[0].failed_at,
            TOTAL_CAP as i64 + 49,
            "the newest are kept"
        );
    }

    #[test]
    fn filters_combine_and_reason_is_a_case_insensitive_substring() {
        let all = sample();
        let by_queue = FailedFilter {
            queue: Some("emails".into()),
            ..Default::default()
        };
        assert_eq!(page_of(&all, &by_queue, 0, 50).1, 3);

        let timeouts = FailedFilter {
            reason: Some("etimedout".into()),
            ..Default::default()
        };
        let (page, total) = page_of(&all, &timeouts, 0, 50);
        assert_eq!(total, 2);
        assert_eq!(page[0].job_id, "3");

        let both = FailedFilter {
            name: Some("send-email".into()),
            reason: Some("ETIMEDOUT".into()),
            ..Default::default()
        };
        assert_eq!(page_of(&all, &both, 0, 50).1, 1);

        // A job without a reason never matches a reason filter; blank filters are ignored.
        let blank = FailedFilter {
            queue: Some("  ".into()),
            reason: Some("".into()),
            ..Default::default()
        };
        assert_eq!(page_of(&all, &blank, 0, 50).1, 5);
    }

    #[test]
    fn pagination_runs_on_the_filtered_set() {
        let all = sample();
        let none = FailedFilter::default();
        let (first, total) = page_of(&all, &none, 0, 2);
        assert_eq!(total, 5);
        assert_eq!(
            first.iter().map(|c| c.job_id.as_str()).collect::<Vec<_>>(),
            vec!["3", "2"]
        );
        let (last, _) = page_of(&all, &none, 4, 2);
        assert_eq!(last.len(), 1);
        let (beyond, _) = page_of(&all, &none, 50, 2);
        assert!(beyond.is_empty());
        // Page size is clamped.
        assert_eq!(page_of(&all, &none, 0, 0).0.len(), 1);
    }

    #[test]
    fn types_are_grouped_per_queue_and_carry_the_latest_reason() {
        let all = sample();
        let types = summarise_types(&all, None);
        assert_eq!(types[0].name, "send-email");
        assert_eq!(types[0].count, 2);
        assert_eq!(
            types[0].last_reason.as_deref(),
            Some("RateLimitError"),
            "job 2 is newer than job 1"
        );
        assert_eq!(types.len(), 4);

        let only_reports = summarise_types(&all, Some("reports"));
        assert_eq!(only_reports.len(), 1);
        assert_eq!(only_reports[0].name, "daily-report");
    }

    // Real reasons, as BullMQ workers write them.
    #[test]
    fn signatures_normalise_the_values_of_real_errors() {
        let cases = [
            ("connect ETIMEDOUT 10.0.4.12:5432", "connect ETIMEDOUT <ip>"),
            (
                "connect ECONNREFUSED 127.0.0.1:6379",
                "connect ECONNREFUSED <ip>",
            ),
            (
                "Request failed with status code 503",
                "Request failed with status code <n>",
            ),
            (
                "ValidationError: \"email\" must be a valid email",
                "ValidationError: <str> must be a valid email",
            ),
            (
                "TypeError: Cannot read properties of undefined (reading 'id')",
                "TypeError: Cannot read properties of undefined (reading <str>)",
            ),
            (
                "RateLimitError: 429 Too Many Requests, retry after 30s",
                "RateLimitError: <n> Too Many Requests, retry after <n>s",
            ),
            (
                "fetch failed: GET https://api.example.com/v1/users/4821?expand=org returned 502",
                "fetch failed: GET <url> returned <n>",
            ),
            (
                "Order 65f1c2a9e4b0a1d2c3e4f5a6 not found",
                "Order <hex> not found",
            ),
            (
                "Tenant 3f2b8c1e-9a4d-4e7b-8c2f-1a2b3c4d5e6f has no plan",
                "Tenant <id> has no plan",
            ),
            (
                "Lock expired at 2026-10-26T18:31:12.000Z for job 1232",
                "Lock expired at <time> for job <n>",
            ),
            ("Segfault at 0x7ffd5e8c", "Segfault at <hex>"),
            (
                "Missing key job_12345 in worker-7",
                "Missing key job_<n> in worker-7",
            ),
            (
                "Job   stalled\tmore than allowable limit",
                "Job stalled more than allowable limit",
            ),
            ("Can't decode utf8 payload", "Can't decode utf8 payload"),
        ];
        for (reason, want) in cases {
            assert_eq!(failure_signature(reason), want, "{reason}");
        }
    }

    #[test]
    fn the_signature_is_the_first_line_and_bounded() {
        assert_eq!(
            failure_signature("\n  Error: boom 42\n    at worker.js:12:5"),
            "Error: boom <n>"
        );
        assert_eq!(failure_signature(""), "");
        let long = "x".repeat(2000);
        assert_eq!(
            failure_signature(&long).chars().count(),
            MAX_SIGNATURE_CHARS
        );
    }

    #[test]
    fn same_cause_with_different_values_groups_together() {
        assert_eq!(
            failure_signature("connect ETIMEDOUT 10.0.4.12:5432"),
            failure_signature("connect ETIMEDOUT 10.0.9.200:5433"),
        );
        assert_ne!(
            failure_signature("connect ETIMEDOUT 10.0.4.12:5432"),
            failure_signature("connect ECONNREFUSED 10.0.4.12:5432"),
        );
    }

    #[test]
    fn causes_are_counted_ranked_and_carry_their_jobs() {
        let hour = HOUR_MS;
        let now = 100 * hour;
        let all = merge_candidates(vec![
            c(
                "emails",
                "1",
                now - 30 * hour,
                "send-email",
                "connect ETIMEDOUT 10.0.0.42:587",
            ),
            c(
                "emails",
                "2",
                now - 2 * hour,
                "send-email",
                "connect ETIMEDOUT 10.0.0.43:587",
            ),
            c(
                "emails",
                "3",
                now - hour / 2,
                "send-reminder",
                "connect ETIMEDOUT 10.0.0.44:587",
            ),
            c(
                "reports",
                "9",
                now - hour,
                "daily-report",
                "Request failed with status code 503",
            ),
            c("images", "7", now - 3 * hour, "resize", ""),
        ]);
        let causes = group_by_cause(&all, None, now);
        assert_eq!(causes.len(), 3);
        let top = &causes[0];
        assert_eq!(top.signature, "connect ETIMEDOUT <ip>");
        assert_eq!(top.count, 3);
        assert_eq!(
            top.sample, "connect ETIMEDOUT 10.0.0.44:587",
            "the newest reason"
        );
        assert_eq!(top.first_at, now - 30 * hour);
        assert_eq!(top.last_at, now - hour / 2);
        assert_eq!(top.job_names, vec!["send-email", "send-reminder"]);
        assert_eq!(top.queue_names, vec!["emails"]);
        assert_eq!(
            top.jobs.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(),
            vec!["3", "2", "1"],
            "newest first"
        );
        // Job 1 is older than the trend's 24 hours; 2 and 3 land in their hours.
        assert_eq!(top.trend.len(), TREND_HOURS);
        assert_eq!(top.trend.iter().sum::<u32>(), 2);
        assert_eq!(top.trend[TREND_HOURS - 1], 1);
        assert_eq!(top.trend[TREND_HOURS - 2], 1);

        // No reason is a cause of its own, with an empty signature.
        assert!(causes
            .iter()
            .any(|c| c.signature.is_empty() && c.count == 1));

        let reports = group_by_cause(&all, Some("reports"), now);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].signature, "Request failed with status code <n>");
    }

    #[test]
    fn the_signature_filter_selects_one_cause() {
        let all = sample();
        let timeouts = FailedFilter {
            signature: Some("connect ETIMEDOUT <ip>".into()),
            ..Default::default()
        };
        assert_eq!(page_of(&all, &timeouts, 0, 50).1, 2);
        let no_reason = FailedFilter {
            signature: Some(String::new()),
            ..Default::default()
        };
        let (page, total) = page_of(&all, &no_reason, 0, 50);
        assert_eq!(total, 1);
        assert_eq!(page[0].job_id, "7");
    }
}
