//! BullMQ v6 decoder + key builder.
//!
//! ## Differences from v5
//!
//! - **Stalled set is canonical.** v6 promoted `bull:{q}:stalled` to a
//!   first-class set used by stalled-job recovery. Same key shape as v5
//!   but the BullMQ runtime treats it differently; the decoder is
//!   structurally identical.
//! - **Marker-job pattern.** v6 introduced a sentinel "marker" entry at
//!   the head of the `wait`, `prioritized`, and `delayed` lists/sets so
//!   workers can short-circuit empty-list checks without a separate
//!   roundtrip. **The marker is NOT a real job id** — list iterators
//!   must filter it. The decoder doesn't see markers
//!   directly (it operates on a job hash, not on the list), but the
//!   list-iterator code MUST consult the per-version marker convention.
//!   Convention: BullMQ v6 marker is the literal byte `0` (a single-byte
//!   string). When iterating a list, skip any element matching the
//!   marker token before treating the rest as job ids.
//! - **Field encoding** is identical to v5 for the fields Ranch consumes
//!   (name, data, opts, attemptsMade, timestamp, processedOn,
//!   finishedOn, delay, failedReason, stacktrace, returnvalue, parentKey).
//!   Future v6 minors that drift gain a new `vN.rs`.
//!
//! Even when v6 encoding matches v5 byte-for-byte, we keep the decoders
//! **separate**. One BullMQ minor = one decoder branch. Future
//! divergence ships in one module without rippling.
//!
//! ## Marker constant
//!
//! [`MARKER_TOKEN`] is the byte representation of the v6 marker entry —
//! exposed for the list-iterator code so the convention lives
//! exactly once in the codebase.

use std::collections::HashMap;

use super::decode_helpers::{
    get_json_str, get_json_str_opt, get_parsed, get_parsed_first_or, get_parsed_opt,
    get_stacktrace, get_str, get_str_opt, infer_state,
};
use super::domain::{Job, JobRef, QueueRef};
use super::keys::Keys;
use super::schema::{JobDecoder, SchemaError};

/// BullMQ v6 marker-job sentinel. List iterators MUST filter
/// any list element whose bytes match this constant before treating the
/// rest as job ids. (BullMQ v5 doesn't use markers — its lists contain
/// only real job ids.)
pub const MARKER_TOKEN: &[u8] = b"0";

#[derive(Debug, Clone, Copy, Default)]
pub struct V6Keys;

impl Keys for V6Keys {
    fn wait(&self, q: &QueueRef) -> String {
        format!("{}:{}:wait", q.prefix, q.name)
    }
    fn active(&self, q: &QueueRef) -> String {
        format!("{}:{}:active", q.prefix, q.name)
    }
    fn completed(&self, q: &QueueRef) -> String {
        format!("{}:{}:completed", q.prefix, q.name)
    }
    fn failed(&self, q: &QueueRef) -> String {
        format!("{}:{}:failed", q.prefix, q.name)
    }
    fn delayed(&self, q: &QueueRef) -> String {
        format!("{}:{}:delayed", q.prefix, q.name)
    }
    fn prioritized(&self, q: &QueueRef) -> String {
        format!("{}:{}:prioritized", q.prefix, q.name)
    }
    fn waiting_children(&self, q: &QueueRef) -> String {
        format!("{}:{}:waiting-children", q.prefix, q.name)
    }
    fn stalled(&self, q: &QueueRef) -> String {
        format!("{}:{}:stalled", q.prefix, q.name)
    }
    fn job_hash(&self, jref: &JobRef) -> String {
        format!("{}:{}:{}", jref.queue.prefix, jref.queue.name, jref.id)
    }
    fn meta(&self, q: &QueueRef) -> String {
        format!("{}:{}:meta", q.prefix, q.name)
    }
    fn paused(&self, q: &QueueRef) -> String {
        format!("{}:{}:paused", q.prefix, q.name)
    }
    fn repeat(&self, q: &QueueRef) -> String {
        format!("{}:{}:repeat", q.prefix, q.name)
    }
    fn logs(&self, jref: &JobRef) -> String {
        format!("{}:{}:{}:logs", jref.queue.prefix, jref.queue.name, jref.id)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct V6Decoder;

impl JobDecoder for V6Decoder {
    fn decode_job(
        &self,
        jref: JobRef,
        fields: HashMap<String, Vec<u8>>,
    ) -> Result<Job, SchemaError> {
        // Field encoding matches v5 for the current fields; we keep
        // the decoder structurally separate so future v6 minors can drift
        // without touching v5.rs.
        let name = get_str(&fields, "name")?;
        let data = get_json_str(&fields, "data")?;
        let opts = get_json_str(&fields, "opts")?;
        // See v5.rs — BullMQ uses the short form `atm` in current
        // versions; we accept both for backwards compat with fixtures.
        let attempts_made: u32 = get_parsed_first_or(&fields, &["atm", "attemptsMade"], 0)?;
        let timestamp: i64 = get_parsed(&fields, "timestamp")?;
        let processed_on: Option<i64> = get_parsed_opt(&fields, "processedOn");
        let finished_on: Option<i64> = get_parsed_opt(&fields, "finishedOn");
        let delay: Option<i64> = get_parsed_opt(&fields, "delay");
        let failed_reason = get_str_opt(&fields, "failedReason");
        let stacktrace = get_stacktrace(&fields);
        let return_value = get_json_str_opt(&fields, "returnvalue");
        // BullMQ initialises `progress` to 0; only a reported value is kept.
        let progress = get_json_str_opt(&fields, "progress").filter(|p| p.trim() != "0");
        let parent_key = get_str_opt(&fields, "parentKey");
        let processed_by = get_str_opt(&fields, "pb").filter(|s| !s.is_empty());

        let state = infer_state(
            &failed_reason,
            finished_on,
            processed_on,
            delay,
            parent_key.as_deref(),
        );

        Ok(Job {
            job_ref: jref,
            name,
            state,
            data,
            opts,
            attempts_made,
            timestamp,
            processed_on,
            finished_on,
            delay,
            failed_reason,
            stacktrace,
            return_value,
            parent_key,
            progress,
            processed_by,
            lock_ttl_ms: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::bullmq_schema::JobState;

    fn qref(prefix: &str, name: &str) -> QueueRef {
        QueueRef {
            prefix: prefix.into(),
            name: name.into(),
        }
    }

    fn jref(prefix: &str, queue: &str, id: &str) -> JobRef {
        JobRef {
            queue: qref(prefix, queue),
            id: id.into(),
        }
    }

    fn fields(pairs: &[(&str, &str)]) -> HashMap<String, Vec<u8>> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn keys_match_documented_shapes() {
        let k = V6Keys;
        let q = qref("bull", "reports");
        let j = jref("bull", "reports", "99");
        assert_eq!(k.wait(&q), "bull:reports:wait");
        assert_eq!(k.prioritized(&q), "bull:reports:prioritized");
        assert_eq!(k.stalled(&q), "bull:reports:stalled");
        assert_eq!(k.job_hash(&j), "bull:reports:99");
        assert_eq!(k.logs(&j), "bull:reports:99:logs");
    }

    #[test]
    fn keys_custom_prefix_propagates() {
        let k = V6Keys;
        let q = qref("ranch_test", "q");
        assert_eq!(k.meta(&q), "ranch_test:q:meta");
    }

    #[test]
    fn marker_token_is_single_byte_zero() {
        assert_eq!(MARKER_TOKEN, b"0");
    }

    /// See v5.rs: `storeJob.lua` writes no `atm`; a job that has never run must decode.
    #[test]
    fn decoder_accepts_a_job_exactly_as_bullmq_adds_it() {
        let f = fields(&[
            ("name", "process-report"),
            ("data", r#"{"reportId":"r-42"}"#),
            ("opts", r#"{"attempts":3}"#),
            ("timestamp", "1700000000000"),
            ("delay", "0"),
            ("priority", "0"),
        ]);
        let job = V6Decoder
            .decode_job(jref("bull", "reports", "1"), f)
            .unwrap();
        assert_eq!(job.attempts_made, 0);
        assert_eq!(job.name, "process-report");
    }

    #[test]
    fn decoder_parses_minimal_waiting_job() {
        let f = fields(&[
            ("name", "process-report"),
            ("data", r#"{"reportId":"r-42"}"#),
            ("opts", r#"{}"#),
            ("attemptsMade", "0"),
            ("timestamp", "1700000000000"),
        ]);
        let job = V6Decoder
            .decode_job(jref("bull", "reports", "1"), f)
            .unwrap();
        assert_eq!(job.name, "process-report");
        assert_eq!(job.state, JobState::Waiting);
        // `data` is a raw JSON string; parse on demand for assertions.
        let data: serde_json::Value = serde_json::from_str(&job.data).unwrap();
        assert_eq!(data["reportId"], "r-42");
    }

    #[test]
    fn decoder_handles_flow_child_with_parent_key() {
        let f = fields(&[
            ("name", "child"),
            ("data", "{}"),
            ("opts", "{}"),
            ("attemptsMade", "0"),
            ("timestamp", "0"),
            ("parentKey", "bull:parent-queue:42"),
        ]);
        let job = V6Decoder
            .decode_job(jref("bull", "child-queue", "10"), f)
            .unwrap();
        assert_eq!(job.state, JobState::WaitingChildren);
        assert_eq!(job.parent_key.as_deref(), Some("bull:parent-queue:42"));
    }

    #[test]
    fn decoder_handles_delayed_job() {
        let f = fields(&[
            ("name", "n"),
            ("data", "{}"),
            ("opts", "{}"),
            ("attemptsMade", "0"),
            ("timestamp", "0"),
            ("delay", "60000"),
        ]);
        let job = V6Decoder.decode_job(jref("bull", "q", "1"), f).unwrap();
        assert_eq!(job.state, JobState::Delayed);
        assert_eq!(job.delay, Some(60000));
    }

    #[test]
    fn decoder_returns_missing_field_when_required_absent() {
        // Missing `timestamp` (required).
        let f = fields(&[
            ("name", "n"),
            ("data", "{}"),
            ("opts", "{}"),
            ("attemptsMade", "0"),
        ]);
        let err = V6Decoder.decode_job(jref("bull", "q", "x"), f).unwrap_err();
        assert!(matches!(err, SchemaError::MissingField { ref field } if field == "timestamp"));
    }
}
