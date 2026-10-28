//! Shared decoder helpers used by both `v5.rs` and `v6.rs`.
//!
//! These functions translate raw `HashMap<String, Vec<u8>>` lookups (the
//! shape Ranch will receive from `redis::cmd("HGETALL")`)
//! into typed `Result`s with `SchemaError` variants.

use std::collections::HashMap;

use super::domain::{states, JobState};
use super::schema::SchemaError;

/// Required UTF-8 string field. Missing → `MissingField`. Present but
/// non-utf8 → `InvalidField`.
pub(super) fn get_str(fields: &HashMap<String, Vec<u8>>, key: &str) -> Result<String, SchemaError> {
    let bytes = fields.get(key).ok_or_else(|| SchemaError::MissingField {
        field: key.to_string(),
    })?;
    std::str::from_utf8(bytes)
        .map(|s| s.to_string())
        .map_err(|e| SchemaError::InvalidField {
            field: key.to_string(),
            reason: format!("non-utf8: {e}"),
        })
}

/// Optional UTF-8 string field. Missing → `None`. Non-utf8 also collapses
/// to `None` (we treat non-utf8 optional fields as absent, since we don't
/// have a graceful way to surface them otherwise; feature code can spot
/// the absence and re-fetch the raw bytes if it really needs them).
pub(super) fn get_str_opt(fields: &HashMap<String, Vec<u8>>, key: &str) -> Option<String> {
    fields
        .get(key)
        .and_then(|b| std::str::from_utf8(b).ok().map(|s| s.to_string()))
}

/// Required numeric field, parsed via `FromStr`. Missing → `MissingField`.
/// Non-numeric → `InvalidField`.
pub(super) fn get_parsed<T>(fields: &HashMap<String, Vec<u8>>, key: &str) -> Result<T, SchemaError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let s = get_str(fields, key)?;
    s.parse().map_err(|e: T::Err| SchemaError::InvalidField {
        field: key.to_string(),
        reason: format!("{e}"),
    })
}

/// Optional numeric field. Missing or unparseable → `None`. Used for the
/// fields BullMQ stores conditionally (e.g. `delay`, `processedOn`,
/// `finishedOn` aren't always present).
pub(super) fn get_parsed_opt<T>(fields: &HashMap<String, Vec<u8>>, key: &str) -> Option<T>
where
    T: std::str::FromStr,
{
    get_str_opt(fields, key).and_then(|s| s.parse().ok())
}

/// Numeric field with multiple candidate keys (tried in order), and a value
/// for when none is there. Used for fields BullMQ renamed across versions,
/// e.g. `attemptsMade` in older minors became `atm` in v5.20+ as part of a
/// hash-key length-optimization pass. We try the short form first (more
/// common in real-world data) then fall back to the long form for fixtures /
/// older deployments.
///
/// All keys missing → `default`: BullMQ does not write `atm` when it adds a
/// job (`storeJob.lua` stores name, data, opts, timestamp, delay, priority),
/// only when an attempt ends. Requiring it made every job that had never run
/// (waiting, delayed, prioritized, a first active run) undecodable, and the
/// lists of a busy production queue came back empty.
/// Key present but unparseable → `InvalidField` reporting the matched key.
pub(super) fn get_parsed_first_or<T>(
    fields: &HashMap<String, Vec<u8>>,
    keys: &[&str],
    default: T,
) -> Result<T, SchemaError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    for k in keys {
        if let Some(s) = get_str_opt(fields, k) {
            return s.parse().map_err(|e: T::Err| SchemaError::InvalidField {
                field: (*k).to_string(),
                reason: format!("{e}"),
            });
        }
    }
    Ok(default)
}

/// Required JSON-encoded field: VALIDATES parseability but returns the
/// original raw string for transmission across the boundary. Catching
/// malformed JSON here means the user gets a typed `InvalidJson` error
/// instead of a frontend `JSON.parse` failure later in the drill-down view.
///
/// Design choice: store as `String` on the wire (not
/// `serde_json::Value`) because specta's `serde_json::Value` rendering
/// produces unusable TS types. Frontend calls `JSON.parse(job.data)` on
/// demand when it needs structured access.
pub(super) fn get_json_str(
    fields: &HashMap<String, Vec<u8>>,
    key: &str,
) -> Result<String, SchemaError> {
    let s = get_str(fields, key)?;
    let _: serde_json::Value = serde_json::from_str(&s).map_err(|e| SchemaError::InvalidJson {
        field: key.to_string(),
        reason: format!("{e}"),
    })?;
    Ok(s)
}

/// Optional JSON field. Missing → `None`. Non-utf8 → `None`. Invalid JSON
/// → `None` (treat malformed optional JSON as absent rather than failing
/// the whole decode). `returnvalue` is the main user of this; a job
/// without a parseable return value still has every other field intact.
pub(super) fn get_json_str_opt(fields: &HashMap<String, Vec<u8>>, key: &str) -> Option<String> {
    let s = get_str_opt(fields, key)?;
    // Validate parseability; drop the field on parse error.
    serde_json::from_str::<serde_json::Value>(&s).ok()?;
    Some(s)
}

/// Decode the `stacktrace` field. BullMQ stores it as a JSON-encoded
/// `String[]`. If parsing fails, fall back to a single-line vec
/// containing the raw string (better than dropping diagnostic data).
/// Missing field → empty vec.
pub(super) fn get_stacktrace(fields: &HashMap<String, Vec<u8>>) -> Vec<String> {
    let Some(s) = get_str_opt(fields, "stacktrace") else {
        return Vec::new();
    };
    serde_json::from_str(&s).unwrap_or_else(|_| vec![s])
}

/// Heuristic state inference from job-hash fields alone. The decoder
/// returns this as `Job.state`; callers performing a list-membership
/// query can override the result.
///
/// Order of precedence:
///   1. `failed_reason` set → `Failed`
///   2. `finished_on` set + no `failed_reason` → `Completed`
///   3. `processed_on` set + no `finished_on` → `Active`
///   4. `delay > 0` and not yet processed → `Delayed`
///   5. `parent_key` set and not finished → `WaitingChildren`
///   6. otherwise → `Waiting`
pub(super) fn infer_state(
    failed_reason: &Option<String>,
    finished_on: Option<i64>,
    processed_on: Option<i64>,
    delay: Option<i64>,
    parent_key: Option<&str>,
) -> JobState {
    if failed_reason.is_some() {
        return JobState::Failed;
    }
    if finished_on.is_some() {
        return JobState::Completed;
    }
    if processed_on.is_some() {
        return JobState::Active;
    }
    if matches!(delay, Some(d) if d > 0) {
        return JobState::Delayed;
    }
    if parent_key.is_some() {
        return states::WAITING_CHILDREN;
    }
    JobState::Waiting
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> HashMap<String, Vec<u8>> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_bytes().to_vec()))
            .collect()
    }

    // ── State inference matrix ────────────────────────────────────────

    #[test]
    fn infer_failed_when_failed_reason_set() {
        assert_eq!(
            infer_state(&Some("oops".into()), Some(100), Some(50), None, None),
            JobState::Failed
        );
    }

    #[test]
    fn infer_completed_when_finished_on_set() {
        assert_eq!(
            infer_state(&None, Some(100), Some(50), None, None),
            JobState::Completed
        );
    }

    #[test]
    fn infer_active_when_processed_on_set() {
        assert_eq!(
            infer_state(&None, None, Some(50), None, None),
            JobState::Active
        );
    }

    #[test]
    fn infer_delayed_when_delay_positive() {
        assert_eq!(
            infer_state(&None, None, None, Some(5000), None),
            JobState::Delayed
        );
    }

    #[test]
    fn infer_waiting_when_delay_zero() {
        assert_eq!(
            infer_state(&None, None, None, Some(0), None),
            JobState::Waiting
        );
    }

    #[test]
    fn infer_waiting_children_when_parent_set() {
        assert_eq!(
            infer_state(&None, None, None, None, Some("bull:flow:42")),
            states::WAITING_CHILDREN
        );
    }

    #[test]
    fn infer_waiting_default() {
        assert_eq!(
            infer_state(&None, None, None, None, None),
            JobState::Waiting
        );
    }

    // ── Helper functions ─────────────────────────────────────────────

    #[test]
    fn get_str_returns_missing_field_when_absent() {
        let f = fields(&[]);
        let err = get_str(&f, "name").unwrap_err();
        assert!(matches!(err, SchemaError::MissingField { ref field } if field == "name"));
    }

    #[test]
    fn get_parsed_returns_invalid_field_when_unparseable() {
        let f = fields(&[("attemptsMade", "abc")]);
        let err = get_parsed::<u32>(&f, "attemptsMade").unwrap_err();
        assert!(
            matches!(err, SchemaError::InvalidField { ref field, .. } if field == "attemptsMade")
        );
    }

    #[test]
    fn get_json_str_returns_invalid_json_when_malformed() {
        let f = fields(&[("data", "not json")]);
        let err = get_json_str(&f, "data").unwrap_err();
        assert!(matches!(err, SchemaError::InvalidJson { ref field, .. } if field == "data"));
    }

    #[test]
    fn get_json_str_returns_raw_string_on_valid_json() {
        let f = fields(&[("data", r#"{"to":"a@b.c"}"#)]);
        let s = get_json_str(&f, "data").unwrap();
        assert_eq!(s, r#"{"to":"a@b.c"}"#);
    }

    #[test]
    fn get_json_str_opt_drops_malformed_silently() {
        let f = fields(&[("returnvalue", "not json either")]);
        assert!(get_json_str_opt(&f, "returnvalue").is_none());
    }

    #[test]
    fn get_stacktrace_falls_back_to_single_line_on_invalid_json() {
        let f = fields(&[("stacktrace", "raw error message")]);
        let stack = get_stacktrace(&f);
        assert_eq!(stack, vec!["raw error message".to_string()]);
    }

    #[test]
    fn get_stacktrace_parses_json_array() {
        let f = fields(&[("stacktrace", r#"["line 1","line 2","line 3"]"#)]);
        let stack = get_stacktrace(&f);
        assert_eq!(stack, vec!["line 1", "line 2", "line 3"]);
    }
}
