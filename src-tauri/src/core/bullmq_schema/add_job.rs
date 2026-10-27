//! Add a job.
//!
//! # How Ranch writes what BullMQ writes with Lua
//!
//! The two options on the table were hand-rolled `MULTI/EXEC` pipelines and
//! embedding the upstream scripts. Neither fits:
//!
//!   - `MULTI/EXEC` cannot *read then decide*: adding a job must look at
//!     `meta.paused` / `meta.concurrency` to pick the list and the marker,
//!     and compute the delayed score from what is already in the set. Done
//!     in two steps it races with every producer and with pause/resume.
//!   - The upstream scripts take msgpack-packed arguments whose layout, and
//!     the scripts' key lists, change between minor versions (the `-9` in
//!     `addStandardJob-9` is the key count). Embedding them pins Ranch to
//!     one BullMQ build and drags ~10 include files for features Ranch does
//!     not expose (flows, deduplication, repeat).
//!
//! **Decision: a Ranch-owned Lua script per complex write**, a faithful port
//! of the upstream semantics for the subset Ranch exposes, plain arguments,
//! run with `EVALSHA`. The Redis *data layout* is the stable contract between
//! BullMQ versions — that is what the script writes — and the proof is a
//! test where real BullMQ 5.x and 6.x workers process what Ranch added
//! (`tests/add_job_compat_live.rs`). Any Redis that runs BullMQ allows
//! scripts: BullMQ itself is made of them.
//!
//! This module is the pure half: validation, the stored `opts`, placement.

use serde::{Deserialize, Serialize};
use specta::Type;

/// The script. See its header for `KEYS` / `ARGV`.
pub const ADD_JOB_LUA: &str = include_str!("lua/add_job.lua");

/// BullMQ's own ceiling (`Job.validateOptions`).
pub const MAX_PRIORITY: u32 = 2_097_152;
/// Ten years — a typo guard, not a BullMQ limit.
pub const MAX_DELAY_MS: u64 = 10 * 365 * 24 * 60 * 60 * 1000;
pub const MAX_ATTEMPTS: u32 = 1_000;
pub const MAX_NAME_LEN: usize = 256;
/// Payload cap. Redis would take 512 MB; a hand-typed job never needs it.
pub const MAX_DATA_BYTES: usize = 1024 * 1024;

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

/// BullMQ's built-in backoff strategies (`opts.backoff`).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Backoff {
    pub kind: BackoffKind,
    pub delay_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BackoffKind {
    Fixed,
    Exponential,
}

/// Where BullMQ puts a new job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// `wait` — or `paused` when the queue is paused (decided in the script).
    Standard,
    Delayed,
    Prioritized,
}

/// A validated job, ready for the script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedJob {
    pub name: String,
    /// The payload, validated and minified — key order and number spelling
    /// exactly as typed, like `JSON.stringify` of the same object would be.
    pub data: String,
    /// The `opts` hash field: only what was asked for, like BullMQ stores it.
    pub opts: String,
    pub delay_ms: u64,
    pub priority: u32,
    pub lifo: bool,
    /// A custom id, or `None` for the queue's counter.
    pub job_id: Option<String>,
}

impl PreparedJob {
    /// Same precedence as `Job.addJob`: delay, then priority, then standard.
    pub fn placement(&self) -> Placement {
        if self.delay_ms > 0 {
            Placement::Delayed
        } else if self.priority > 0 {
            Placement::Prioritized
        } else {
            Placement::Standard
        }
    }
}

impl NewJob {
    /// Validate and normalize. The error is a sentence for the form / agent.
    pub fn prepare(&self) -> Result<PreparedJob, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Job name is required".into());
        }
        if name.chars().count() > MAX_NAME_LEN {
            return Err(format!("Job name is longer than {MAX_NAME_LEN} characters"));
        }
        // BullMQ rejects it too: `:` separates the segments of its keys.
        if name.contains(':') {
            return Err("Job name cannot contain ':'".into());
        }
        if self.data.len() > MAX_DATA_BYTES {
            return Err(format!(
                "Payload is larger than {} KB",
                MAX_DATA_BYTES / 1024
            ));
        }
        serde_json::from_str::<serde::de::IgnoredAny>(self.data.trim())
            .map_err(|e| format!("Payload is not valid JSON: {e}"))?;
        if self.delay_ms > MAX_DELAY_MS {
            return Err("Delay is longer than 10 years".into());
        }
        if self.priority > MAX_PRIORITY {
            return Err(format!("Priority must be between 0 and {MAX_PRIORITY}"));
        }
        if let Some(attempts) = self.attempts {
            if attempts == 0 || attempts > MAX_ATTEMPTS {
                return Err(format!("Attempts must be between 1 and {MAX_ATTEMPTS}"));
            }
        }
        if let Some(backoff) = &self.backoff {
            if backoff.delay_ms > MAX_DELAY_MS {
                return Err("Backoff is longer than 10 years".into());
            }
        }
        let job_id = match self.job_id.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(id) => {
                // BullMQ's own rules (`Job.validateOptions`): its counter
                // owns the whole numbers, and `:` separates key segments.
                if id.parse::<u64>().is_ok() {
                    return Err(
                        "A custom job ID cannot be a whole number: BullMQ numbers its own jobs"
                            .into(),
                    );
                }
                if id.contains(':') {
                    return Err("A job ID cannot contain ':'".into());
                }
                if id.chars().count() > MAX_NAME_LEN {
                    return Err(format!("Job ID is longer than {MAX_NAME_LEN} characters"));
                }
                Some(id.to_string())
            }
        };

        let mut opts = serde_json::Map::new();
        if let Some(attempts) = self.attempts {
            opts.insert("attempts".into(), attempts.into());
        }
        if self.delay_ms > 0 {
            opts.insert("delay".into(), self.delay_ms.into());
        }
        if self.priority > 0 {
            opts.insert("priority".into(), self.priority.into());
        }
        if self.lifo {
            opts.insert("lifo".into(), true.into());
        }
        if let Some(backoff) = &self.backoff {
            let kind = match backoff.kind {
                BackoffKind::Fixed => "fixed",
                BackoffKind::Exponential => "exponential",
            };
            opts.insert(
                "backoff".into(),
                serde_json::json!({"type": kind, "delay": backoff.delay_ms}),
            );
        }
        if let Some(id) = &job_id {
            opts.insert("jobId".into(), id.clone().into());
        }
        if self.remove_on_complete {
            opts.insert("removeOnComplete".into(), true.into());
        }
        if self.remove_on_fail {
            opts.insert("removeOnFail".into(), true.into());
        }

        Ok(PreparedJob {
            name: name.to_string(),
            data: minify_json(self.data.trim()),
            opts: serde_json::Value::Object(opts).to_string(),
            delay_ms: self.delay_ms,
            priority: self.priority,
            lifo: self.lifo,
            job_id,
        })
    }
}

/// Drop the whitespace between tokens of an already-validated JSON text.
/// Parsing and re-serializing would do it too, but reorders the keys and
/// respells the numbers (`1.0` → `1.0`/`1`, `1e3` → `1000.0`): the payload a
/// worker receives should be the one that was typed.
pub(super) fn minify_json(valid_json: &str) -> String {
    let mut out = String::with_capacity(valid_json.len());
    let mut in_string = false;
    let mut escaped = false;
    for c in valid_json.chars() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if !matches!(c, ' ' | '\t' | '\n' | '\r') {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(name: &str, data: &str) -> NewJob {
        NewJob {
            name: name.into(),
            data: data.into(),
            delay_ms: 0,
            attempts: None,
            priority: 0,
            lifo: false,
            backoff: None,
            job_id: None,
            remove_on_complete: false,
            remove_on_fail: false,
        }
    }

    #[test]
    fn a_plain_job_stores_empty_opts_and_compact_data() {
        let p = job("  send-email ", "{ \"to\": \"a@b.c\",\n \"n\": 1 }")
            .prepare()
            .expect("valid");
        assert_eq!(p.name, "send-email");
        assert_eq!(
            p.data, r#"{"to":"a@b.c","n":1}"#,
            "key order kept, whitespace dropped"
        );
        assert_eq!(p.opts, "{}");
        assert_eq!(p.placement(), Placement::Standard);
    }

    #[test]
    fn minifying_keeps_strings_order_and_number_spelling() {
        let typed = "{\n  \"b\": \"two  spaces, a \\\" quote and a \\\\\",\n  \"a\": [1.50, 1e3, true, null],\t\"c\": {}\r\n}";
        let p = job("x", typed).prepare().expect("valid");
        assert_eq!(
            p.data,
            r#"{"b":"two  spaces, a \" quote and a \\","a":[1.50,1e3,true,null],"c":{}}"#
        );
        // Still the same document.
        let before: serde_json::Value = serde_json::from_str(typed).unwrap();
        let after: serde_json::Value = serde_json::from_str(&p.data).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn opts_hold_only_what_was_asked_for() {
        let p = NewJob {
            delay_ms: 60_000,
            attempts: Some(3),
            priority: 5,
            lifo: true,
            ..job("x", "[1,2]")
        }
        .prepare()
        .expect("valid");
        let opts: serde_json::Value = serde_json::from_str(&p.opts).unwrap();
        assert_eq!(
            opts,
            serde_json::json!({"attempts": 3, "delay": 60000, "priority": 5, "lifo": true})
        );
        assert_eq!(
            p.placement(),
            Placement::Delayed,
            "delay wins over priority"
        );
        let prio = NewJob {
            priority: 5,
            ..job("x", "null")
        };
        assert_eq!(prio.prepare().unwrap().placement(), Placement::Prioritized);
    }

    #[test]
    fn rejects_what_bullmq_or_common_sense_rejects() {
        let cases: Vec<(NewJob, &str)> = vec![
            (job("   ", "{}"), "name is required"),
            (job("a:b", "{}"), "cannot contain ':'"),
            (job(&"n".repeat(257), "{}"), "longer than 256"),
            (job("x", "{not json"), "not valid JSON"),
            (job("x", ""), "not valid JSON"),
            (
                NewJob {
                    priority: MAX_PRIORITY + 1,
                    ..job("x", "{}")
                },
                "Priority must be",
            ),
            (
                NewJob {
                    attempts: Some(0),
                    ..job("x", "{}")
                },
                "Attempts must be",
            ),
            (
                NewJob {
                    delay_ms: MAX_DELAY_MS + 1,
                    ..job("x", "{}")
                },
                "longer than 10 years",
            ),
            (
                job("x", &format!("\"{}\"", "a".repeat(MAX_DATA_BYTES))),
                "larger than",
            ),
        ];
        for (input, expected) in cases {
            let err = input.prepare().expect_err(expected);
            assert!(err.contains(expected), "{err} should mention {expected}");
        }
    }

    #[test]
    fn the_script_is_embedded_and_names_every_key_it_uses() {
        assert!(ADD_JOB_LUA.contains("KEYS[10]"));
        assert!(!ADD_JOB_LUA.contains("KEYS[11]"));
        assert!(!ADD_JOB_LUA.contains("cmsgpack"), "plain arguments only");
    }

    // The options of the Add Job sheet.
    #[test]
    fn backoff_custom_id_and_removal_go_into_opts() {
        let p = NewJob {
            attempts: Some(5),
            backoff: Some(Backoff {
                kind: BackoffKind::Exponential,
                delay_ms: 2_000,
            }),
            job_id: Some("  order-1042 ".into()),
            remove_on_complete: true,
            remove_on_fail: true,
            ..job("x", "{}")
        }
        .prepare()
        .expect("valid");
        assert_eq!(p.job_id.as_deref(), Some("order-1042"));
        let opts: serde_json::Value = serde_json::from_str(&p.opts).unwrap();
        assert_eq!(
            opts,
            serde_json::json!({
                "attempts": 5,
                "backoff": {"type": "exponential", "delay": 2000},
                "jobId": "order-1042",
                "removeOnComplete": true,
                "removeOnFail": true
            })
        );
        // A blank id is the counter's.
        let blank = NewJob {
            job_id: Some("   ".into()),
            ..job("x", "{}")
        };
        assert_eq!(blank.prepare().unwrap().job_id, None);
    }

    #[test]
    fn custom_ids_follow_bullmq_rules() {
        for (id, want) in [("1042", "whole number"), ("a:b", "':'")] {
            let err = NewJob {
                job_id: Some(id.into()),
                ..job("x", "{}")
            }
            .prepare()
            .unwrap_err();
            assert!(err.contains(want), "{id}: {err}");
        }
    }
}
