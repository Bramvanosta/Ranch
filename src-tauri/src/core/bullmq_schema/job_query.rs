//! Server-side job search and sort.
//!
//! Pure logic. Without a query the actor reads one page straight from the
//! state's list / sorted set — cheap, exact, unbounded. With a text search, a
//! reason filter or a custom sort, it must look at the jobs themselves: it
//! scans the state (bounded by [`SCAN_CAP`], newest first), loads the few
//! fields below for every id, and this module filters, sorts and pages.
//! The old filter ran in the browser over the 50 rows already on screen —
//! a job on page 3 could not be found.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Jobs of a state looked at by a query (chunked + cancellable).
pub const SCAN_CAP: usize = 10_000;
/// `HMGET` pipeline size while scanning.
pub const SCAN_CHUNK: usize = 500;
pub const PAGE_SIZES: [u64; 4] = [25, 50, 100, 250];
pub const MAX_PAGE_SIZE: u64 = 250;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum JobSortField {
    /// Job name, then id.
    Name,
    /// `timestamp` — when the job was added.
    QueuedAt,
    /// `finishedOn − processedOn`; jobs without a duration go last.
    Duration,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobSort {
    pub field: JobSortField,
    pub descending: bool,
}

/// What a query needs to know about a job — one `HMGET` per job.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobRow {
    pub id: String,
    pub name: String,
    pub timestamp: Option<i64>,
    pub processed_on: Option<i64>,
    pub finished_on: Option<i64>,
    pub failed_reason: Option<String>,
}

impl JobRow {
    pub fn duration_ms(&self) -> Option<i64> {
        match (self.processed_on, self.finished_on) {
            (Some(p), Some(f)) if f >= p => Some(f - p),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobQuery {
    /// Case-insensitive substring of the job **id or name**.
    pub search: Option<String>,
    /// Case-insensitive substring of the failure reason.
    pub reason: Option<String>,
    pub sort: Option<JobSort>,
}

fn needle(v: &Option<String>) -> Option<String> {
    v.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
}

impl JobQuery {
    /// `false` = the cheap range-read path applies.
    pub fn is_active(&self) -> bool {
        needle(&self.search).is_some() || needle(&self.reason).is_some() || self.sort.is_some()
    }

    pub fn matches(&self, row: &JobRow) -> bool {
        if let Some(s) = needle(&self.search) {
            if !row.id.to_lowercase().contains(&s) && !row.name.to_lowercase().contains(&s) {
                return false;
            }
        }
        if let Some(r) = needle(&self.reason) {
            if !row
                .failed_reason
                .as_deref()
                .is_some_and(|reason| reason.to_lowercase().contains(&r))
            {
                return false;
            }
        }
        true
    }

    /// Filter, sort, and return `(ids of the page, total matching)`. Rows
    /// must come in the state's natural order (what no-sort preserves).
    pub fn run(&self, rows: Vec<JobRow>, offset: u64, limit: u64) -> (Vec<String>, u64) {
        let mut matching: Vec<JobRow> = rows.into_iter().filter(|r| self.matches(r)).collect();
        if let Some(sort) = self.sort {
            // Stable: equal keys keep the natural (newest-first) order.
            matching.sort_by(|a, b| {
                let ord = match sort.field {
                    JobSortField::Name => a
                        .name
                        .to_lowercase()
                        .cmp(&b.name.to_lowercase())
                        .then_with(|| a.id.cmp(&b.id)),
                    JobSortField::QueuedAt => a.timestamp.cmp(&b.timestamp),
                    JobSortField::Duration => {
                        // Jobs without a duration always sink to the end.
                        return match (a.duration_ms(), b.duration_ms()) {
                            (Some(x), Some(y)) if sort.descending => y.cmp(&x),
                            (Some(x), Some(y)) => x.cmp(&y),
                            (Some(_), None) => std::cmp::Ordering::Less,
                            (None, Some(_)) => std::cmp::Ordering::Greater,
                            (None, None) => std::cmp::Ordering::Equal,
                        };
                    }
                };
                if sort.descending {
                    ord.reverse()
                } else {
                    ord
                }
            });
        }
        let total = matching.len() as u64;
        let limit = limit.clamp(1, MAX_PAGE_SIZE) as usize;
        let page = matching
            .into_iter()
            .skip(offset as usize)
            .take(limit)
            .map(|r| r.id)
            .collect();
        (page, total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, name: &str, queued: i64, took: Option<i64>, reason: &str) -> JobRow {
        JobRow {
            id: id.into(),
            name: name.into(),
            timestamp: Some(queued),
            processed_on: took.map(|_| queued + 10),
            finished_on: took.map(|t| queued + 10 + t),
            failed_reason: (!reason.is_empty()).then(|| reason.to_string()),
        }
    }

    fn rows() -> Vec<JobRow> {
        // Natural order = newest first.
        vec![
            row("104", "send-email", 400, Some(30), ""),
            row("103", "Resize", 300, None, ""),
            row("102", "send-reminder", 200, Some(900), "connect ETIMEDOUT"),
            row("101", "resize", 100, Some(5), "RateLimitError"),
        ]
    }

    #[test]
    fn no_query_means_the_cheap_path() {
        assert!(!JobQuery::default().is_active());
        let blank = JobQuery {
            search: Some("   ".into()),
            reason: Some("".into()),
            sort: None,
        };
        assert!(!blank.is_active());
        assert!(JobQuery {
            sort: Some(JobSort {
                field: JobSortField::Name,
                descending: false
            }),
            ..Default::default()
        }
        .is_active());
    }

    #[test]
    fn search_matches_id_or_name_case_insensitively_across_the_whole_set() {
        let by_name = JobQuery {
            search: Some("RESIZE".into()),
            ..Default::default()
        };
        assert_eq!(
            by_name.run(rows(), 0, 50),
            (vec!["103".into(), "101".into()], 2)
        );

        let by_id = JobQuery {
            search: Some("102".into()),
            ..Default::default()
        };
        assert_eq!(by_id.run(rows(), 0, 50).0, vec!["102".to_string()]);

        let reason = JobQuery {
            reason: Some("etimedout".into()),
            ..Default::default()
        };
        assert_eq!(reason.run(rows(), 0, 50).0, vec!["102".to_string()]);
        // A job without a failure reason never matches a reason filter.
        let none = JobQuery {
            search: Some("send-email".into()),
            reason: Some("x".into()),
            sort: None,
        };
        assert_eq!(none.run(rows(), 0, 50).1, 0);
    }

    #[test]
    fn sorts_and_keeps_jobs_without_a_duration_last_both_ways() {
        let sort = |field, descending| JobQuery {
            sort: Some(JobSort { field, descending }),
            ..Default::default()
        };
        assert_eq!(
            sort(JobSortField::Name, false).run(rows(), 0, 50).0,
            vec!["101", "103", "104", "102"],
            "case-insensitive, id breaks ties"
        );
        assert_eq!(
            sort(JobSortField::QueuedAt, false).run(rows(), 0, 50).0,
            vec!["101", "102", "103", "104"]
        );
        assert_eq!(
            sort(JobSortField::Duration, true).run(rows(), 0, 50).0,
            vec!["102", "104", "101", "103"]
        );
        assert_eq!(
            sort(JobSortField::Duration, false).run(rows(), 0, 50).0,
            vec!["101", "104", "102", "103"],
            "103 has no duration: last in ascending order too"
        );
    }

    #[test]
    fn pages_the_filtered_sorted_set_and_clamps_the_size() {
        let q = JobQuery {
            sort: Some(JobSort {
                field: JobSortField::QueuedAt,
                descending: false,
            }),
            ..Default::default()
        };
        assert_eq!(q.run(rows(), 2, 2), (vec!["103".into(), "104".into()], 4));
        assert_eq!(q.run(rows(), 10, 2).0, Vec::<String>::new());
        assert_eq!(q.run(rows(), 0, 0).0.len(), 1);
        let many: Vec<JobRow> = (0..400)
            .map(|i| row(&i.to_string(), "x", i, None, ""))
            .collect();
        assert_eq!(q.run(many, 0, 10_000).0.len(), MAX_PAGE_SIZE as usize);
    }
}
