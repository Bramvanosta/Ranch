//! BullMQ v5 decoder + key builder.
//!
//! The v5 line introduced the `prioritized` sorted-set and refined the
//! `waiting-children` flow-job semantics. Field names match the BullMQ
//! source's `Job` Redis-hash encoding.
//!
//! Reference: BullMQ v5 source `src/keys.ts` and `src/classes/job.ts`
//! (cross-checked against a real BullMQ-backed Redis at impl time).

use std::collections::HashMap;

use super::decode_helpers::{
    get_json_str, get_json_str_opt, get_parsed, get_parsed_first_or, get_parsed_opt,
    get_stacktrace, get_str, get_str_opt, infer_state,
};
use super::domain::{Job, JobRef, QueueRef};
use super::keys::Keys;
use super::schema::{JobDecoder, SchemaError};

/// Zero-sized and `Copy` so callers can hold it in `KeyVariant` or pass it
/// inline in hot paths without allocating. All "state" lives in the
/// BullMQ key conventions, NOT in struct fields.
#[derive(Debug, Clone, Copy, Default)]
pub struct V5Keys;

impl Keys for V5Keys {
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
        // A BullMQ queue id is the base of the queue's keys.
        format!("{}:{}", jref.queue_id, jref.id)
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
        format!("{}:{}:logs", jref.queue_id, jref.id)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct V5Decoder;

impl JobDecoder for V5Decoder {
    fn decode_job(
        &self,
        jref: JobRef,
        fields: HashMap<String, Vec<u8>>,
    ) -> Result<Job, SchemaError> {
        let name = get_str(&fields, "name")?;
        let data = get_json_str(&fields, "data")?;
        let opts = get_json_str(&fields, "opts")?;
        // BullMQ v5.20+ renamed `attemptsMade` → `atm` as part of a hash-key
        // length-optimization pass. Try the short form first (real-world
        // current shape), fall back to the long form (older fixtures).
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
    use crate::bullmq_schema::JobState;

    fn qref(prefix: &str, name: &str) -> QueueRef {
        QueueRef {
            prefix: prefix.into(),
            name: name.into(),
        }
    }

    fn jref(prefix: &str, queue: &str, id: &str) -> JobRef {
        qref(prefix, queue).job(id)
    }

    fn fields(pairs: &[(&str, &str)]) -> HashMap<String, Vec<u8>> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_bytes().to_vec()))
            .collect()
    }

    // ── Keys: every method against a fixed QueueRef + custom prefix ─────────

    #[test]
    fn keys_default_prefix_shapes() {
        let k = V5Keys;
        let q = qref("bull", "email");
        let j = jref("bull", "email", "42");
        assert_eq!(k.wait(&q), "bull:email:wait");
        assert_eq!(k.active(&q), "bull:email:active");
        assert_eq!(k.completed(&q), "bull:email:completed");
        assert_eq!(k.failed(&q), "bull:email:failed");
        assert_eq!(k.delayed(&q), "bull:email:delayed");
        assert_eq!(k.prioritized(&q), "bull:email:prioritized");
        assert_eq!(k.waiting_children(&q), "bull:email:waiting-children");
        assert_eq!(k.stalled(&q), "bull:email:stalled");
        assert_eq!(k.job_hash(&j), "bull:email:42");
        assert_eq!(k.meta(&q), "bull:email:meta");
        assert_eq!(k.paused(&q), "bull:email:paused");
        assert_eq!(k.repeat(&q), "bull:email:repeat");
        assert_eq!(k.logs(&j), "bull:email:42:logs");
    }

    #[test]
    fn keys_custom_prefix_propagates() {
        let k = V5Keys;
        let q = qref("myapp", "queue-1");
        assert_eq!(k.wait(&q), "myapp:queue-1:wait");
        assert_eq!(k.meta(&q), "myapp:queue-1:meta");
    }

    // ── Decoder: minimal happy path ─────────────────────────────────────────

    // The worker that took the job.
    #[test]
    fn decoder_reads_processed_by() {
        let base = [
            ("name", "send-email"),
            ("data", "{}"),
            ("opts", "{}"),
            ("timestamp", "1672531200000"),
            ("processedOn", "1672531201000"),
        ];
        let mut with_pb = base.to_vec();
        with_pb.push(("pb", "worker-emails-1"));
        let job = V5Decoder
            .decode_job(jref("bull", "email", "1"), fields(&with_pb))
            .unwrap();
        assert_eq!(job.processed_by.as_deref(), Some("worker-emails-1"));
        assert_eq!(
            job.lock_ttl_ms, None,
            "the lock is read by the actor, not the decoder"
        );

        let job = V5Decoder
            .decode_job(jref("bull", "email", "1"), fields(&base))
            .unwrap();
        assert_eq!(job.processed_by, None);
    }

    /// What BullMQ's `storeJob.lua` writes when a job is added, and nothing
    /// else: no `atm` until an attempt ends. Requiring it hid every job that
    /// had never run (waiting, delayed, a first active run) from the lists.
    #[test]
    fn decoder_accepts_a_job_exactly_as_bullmq_adds_it() {
        let f = fields(&[
            ("name", "send-email"),
            ("data", r#"{"to":"alice@example.com"}"#),
            ("opts", r#"{"attempts":3,"delay":0}"#),
            ("timestamp", "1672531200000"),
            ("delay", "0"),
            ("priority", "0"),
        ]);
        let job = V5Decoder.decode_job(jref("bull", "email", "1"), f).unwrap();
        assert_eq!(job.attempts_made, 0);
        assert_eq!(job.name, "send-email");
    }

    #[test]
    fn decoder_parses_minimal_waiting_job() {
        let f = fields(&[
            ("name", "send-email"),
            ("data", r#"{"to":"alice@example.com"}"#),
            ("opts", r#"{"attempts":3}"#),
            ("attemptsMade", "0"),
            ("timestamp", "1672531200000"),
        ]);
        let job = V5Decoder.decode_job(jref("bull", "email", "1"), f).unwrap();
        assert_eq!(job.name, "send-email");
        assert_eq!(job.state, JobState::Waiting);
        assert_eq!(job.attempts_made, 0);
        assert_eq!(job.timestamp, 1672531200000);
        // `data` is the validated raw JSON string; frontend parses on demand.
        let data: serde_json::Value = serde_json::from_str(&job.data).unwrap();
        assert_eq!(data["to"], "alice@example.com");
        assert!(job.failed_reason.is_none());
        assert!(job.stacktrace.is_empty());
    }

    #[test]
    fn decoder_parses_failed_job_with_stacktrace() {
        let f = fields(&[
            ("name", "send-email"),
            ("data", r#"{}"#),
            ("opts", r#"{}"#),
            ("attemptsMade", "3"),
            ("timestamp", "1672531200000"),
            ("failedReason", "Connection refused"),
            (
                "stacktrace",
                r#"["Error: Connection refused","  at sendEmail (worker.js:42:11)"]"#,
            ),
        ]);
        let job = V5Decoder.decode_job(jref("bull", "email", "2"), f).unwrap();
        assert_eq!(job.state, JobState::Failed);
        assert_eq!(job.failed_reason.as_deref(), Some("Connection refused"));
        assert_eq!(job.stacktrace.len(), 2);
    }

    #[test]
    fn decoder_returns_missing_field_when_name_absent() {
        let f = fields(&[
            ("data", "{}"),
            ("opts", "{}"),
            ("attemptsMade", "0"),
            ("timestamp", "0"),
        ]);
        let err = V5Decoder
            .decode_job(jref("bull", "email", "3"), f)
            .unwrap_err();
        assert!(matches!(err, SchemaError::MissingField { ref field } if field == "name"));
    }

    #[test]
    fn decoder_returns_invalid_field_for_non_numeric_attempts() {
        let f = fields(&[
            ("name", "n"),
            ("data", "{}"),
            ("opts", "{}"),
            ("attemptsMade", "not-a-number"),
            ("timestamp", "0"),
        ]);
        let err = V5Decoder
            .decode_job(jref("bull", "email", "4"), f)
            .unwrap_err();
        assert!(
            matches!(err, SchemaError::InvalidField { ref field, .. } if field == "attemptsMade")
        );
    }

    #[test]
    fn decoder_returns_invalid_json_for_malformed_data() {
        let f = fields(&[
            ("name", "n"),
            ("data", "not json"),
            ("opts", "{}"),
            ("attemptsMade", "0"),
            ("timestamp", "0"),
        ]);
        let err = V5Decoder
            .decode_job(jref("bull", "email", "5"), f)
            .unwrap_err();
        assert!(matches!(err, SchemaError::InvalidJson { ref field, .. } if field == "data"));
    }

    #[test]
    fn decoder_infers_completed_when_finished_on_set() {
        let f = fields(&[
            ("name", "n"),
            ("data", "{}"),
            ("opts", "{}"),
            ("attemptsMade", "1"),
            ("timestamp", "0"),
            ("processedOn", "1672531200000"),
            ("finishedOn", "1672531205000"),
            ("returnvalue", r#"{"ok":true}"#),
        ]);
        let job = V5Decoder.decode_job(jref("bull", "email", "6"), f).unwrap();
        assert_eq!(job.state, JobState::Completed);
        let rv: serde_json::Value =
            serde_json::from_str(job.return_value.as_ref().unwrap()).unwrap();
        assert_eq!(rv["ok"], true);
    }
}
