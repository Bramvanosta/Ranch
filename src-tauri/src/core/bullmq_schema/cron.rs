//! CRON / repeatable scheduler hash decoder.
//!
//! BullMQ v5+ stores each `upsertJobScheduler` call as:
//!   - One member in `bull:{queue}:repeat` (ZSET, score = next-run ms).
//!   - One hash at `bull:{queue}:repeat:{scheduler_key}` with the
//!     scheduler's metadata (`name`, `pattern` OR `every`, `tz?`,
//!     `offset?`, `ic`, `data?`, `endDate?`).
//!
//! The hash shape is **identical** across V5 and V6 (verified against a
//! BullMQ v5.76 probe and consistent with BullMQ source).
//! No per-version dispatch needed — this module is a single shared
//! decoder rather than a `JobDecoder`-style trait.
//!
//! Unlike `decode_job`, `decode_cron_hash` returns
//! `Option<CronJob>` rather than `Result<_, SchemaError>` because the
//! `list_cron_jobs` handler skips invalid rows with a `tracing::warn!`
//! instead of failing the whole call (per-scheduler resilience —
//! mirrors the `handle_list_jobs` skip-on-decoder-error pattern).

use std::collections::HashMap;

use super::decode_helpers::{get_json_str_opt, get_parsed_opt, get_str, get_str_opt};
use super::domain::CronJob;

/// Decode a `bull:{queue}:repeat:{key}` hash + ZSET score into a typed
/// [`CronJob`]. Returns `None` (with a `tracing::warn!`) on:
///   - Missing required `name` field.
///   - Both `pattern` AND `every` set (BullMQ never stores both).
///   - Neither `pattern` NOR `every` set (invalid scheduler).
///
/// The caller (`handle_list_cron_jobs`) treats `None` as a skip-with-warn.
pub fn decode_cron_hash(
    scheduler_key: String,
    next_run_ms: i64,
    fields: &HashMap<String, Vec<u8>>,
) -> Option<CronJob> {
    let name = match get_str(fields, "name") {
        Ok(s) => s,
        Err(_) => {
            tracing::warn!(
                scheduler_key = %scheduler_key,
                "scheduler hash missing required `name` field; skipping"
            );
            return None;
        }
    };

    let pattern = get_str_opt(fields, "pattern");
    let every: Option<u64> = get_parsed_opt(fields, "every");

    match (pattern.is_some(), every.is_some()) {
        (false, false) => {
            tracing::warn!(
                scheduler_key = %scheduler_key,
                "scheduler hash has neither `pattern` nor `every`; skipping"
            );
            return None;
        }
        (true, true) => {
            tracing::warn!(
                scheduler_key = %scheduler_key,
                "scheduler hash has BOTH `pattern` and `every` (invalid); skipping"
            );
            return None;
        }
        _ => {}
    }

    let tz = get_str_opt(fields, "tz");
    let offset: Option<u64> = get_parsed_opt(fields, "offset");
    let iteration_count: u64 = get_parsed_opt(fields, "ic").unwrap_or(0);
    let end_date: Option<i64> = get_parsed_opt(fields, "endDate");
    // `data` is a JSON-encoded template payload. We validate parseability
    // (so the frontend doesn't get a corrupt blob) but transmit as the
    // raw string, mirroring the `data`/`opts`/`returnvalue`
    // treatment. Unparseable JSON → drop the template, keep the row.
    let template = get_json_str_opt(fields, "data");

    // `paused` field stored as "1" / "0" / absent.
    // Absent → false (BullMQ doesn't set it on creation).
    let paused = matches!(get_str_opt(fields, "paused").as_deref(), Some("1"));

    Some(CronJob {
        key: scheduler_key,
        name,
        pattern,
        every,
        tz,
        offset,
        iteration_count,
        next_run: next_run_ms,
        end_date,
        template,
        paused,
    })
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

    #[test]
    fn decodes_cron_pattern_scheduler() {
        let f = fields(&[
            ("name", "send-daily-report"),
            ("pattern", "0 9 * * *"),
            ("tz", "Europe/Paris"),
            ("offset", "0"),
            ("ic", "5"),
            ("data", r#"{"reportType":"summary"}"#),
        ]);
        let cj = decode_cron_hash("daily-report".into(), 1_778_569_200_000, &f).unwrap();
        assert_eq!(cj.key, "daily-report");
        assert_eq!(cj.name, "send-daily-report");
        assert_eq!(cj.pattern.as_deref(), Some("0 9 * * *"));
        assert_eq!(cj.every, None);
        assert_eq!(cj.tz.as_deref(), Some("Europe/Paris"));
        assert_eq!(cj.iteration_count, 5);
        assert_eq!(cj.next_run, 1_778_569_200_000);
        assert_eq!(cj.template.as_deref(), Some(r#"{"reportType":"summary"}"#));
    }

    #[test]
    fn decodes_fixed_interval_scheduler() {
        let f = fields(&[
            ("name", "ping-health"),
            ("every", "900000"),
            ("offset", "535788"),
            ("ic", "1"),
        ]);
        let cj = decode_cron_hash("every-15m".into(), 1_778_487_835_788, &f).unwrap();
        assert_eq!(cj.pattern, None);
        assert_eq!(cj.every, Some(900_000));
        assert_eq!(cj.tz, None);
        assert_eq!(cj.offset, Some(535_788));
        assert_eq!(cj.iteration_count, 1);
    }

    #[test]
    fn missing_name_yields_none() {
        let f = fields(&[("pattern", "0 9 * * *"), ("ic", "0")]);
        assert!(decode_cron_hash("k".into(), 0, &f).is_none());
    }

    #[test]
    fn neither_pattern_nor_every_yields_none() {
        let f = fields(&[("name", "n"), ("ic", "0")]);
        assert!(decode_cron_hash("k".into(), 0, &f).is_none());
    }

    #[test]
    fn both_pattern_and_every_yields_none() {
        let f = fields(&[("name", "n"), ("pattern", "0 * * * *"), ("every", "60000")]);
        assert!(decode_cron_hash("k".into(), 0, &f).is_none());
    }

    #[test]
    fn missing_ic_defaults_to_zero() {
        let f = fields(&[("name", "n"), ("every", "1000")]);
        let cj = decode_cron_hash("k".into(), 0, &f).unwrap();
        assert_eq!(cj.iteration_count, 0);
    }

    #[test]
    fn invalid_template_json_drops_template_but_keeps_row() {
        let f = fields(&[("name", "n"), ("every", "1000"), ("data", "not json")]);
        let cj = decode_cron_hash("k".into(), 0, &f).unwrap();
        assert_eq!(cj.template, None);
    }

    /// `paused` field is absent by default → false.
    #[test]
    fn paused_defaults_to_false_when_field_absent() {
        let f = fields(&[("name", "n"), ("every", "1000")]);
        let cj = decode_cron_hash("k".into(), 0, &f).unwrap();
        assert!(!cj.paused);
    }

    /// `paused = "1"` → true.
    #[test]
    fn paused_is_true_when_field_is_one() {
        let f = fields(&[("name", "n"), ("every", "1000"), ("paused", "1")]);
        let cj = decode_cron_hash("k".into(), 0, &f).unwrap();
        assert!(cj.paused);
    }

    /// Any non-"1" value treated as false (defensive).
    #[test]
    fn paused_falsy_for_zero_or_other_values() {
        for v in ["0", "false", ""] {
            let f = fields(&[("name", "n"), ("every", "1000"), ("paused", v)]);
            let cj = decode_cron_hash("k".into(), 0, &f).unwrap();
            assert!(!cj.paused, "paused should be false for value: {v:?}");
        }
    }
}
