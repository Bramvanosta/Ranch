//! What agents may see of job data.
//!
//! Settings › AI features › "Job data shared with agents" (kv
//! `ai_share_mode`):
//!
//!   - `everything`: payloads, return values and progress as they are;
//!   - `mask` (the default): every field whose **name** matches one of the
//!     patterns (kv `ai_share_patterns`, default [`DEFAULT_PATTERNS`]) has
//!     its value replaced by `"•••"`, at any depth. Log lines are free text
//!     with no field names: this mode leaves them as they are;
//!   - `names`: names, states and errors only: payloads, return values,
//!     progress, scheduler templates and job logs are not sent at all.
//!
//! Applied in one place, `RanchMcpServer::call_tool`, to the text answer of
//! every tool that carries job data ([`JOB_DATA_TOOLS`]), so no tool can
//! forget it. Failure reasons and stacktraces are errors and stay: the
//! patterns are about field names.

use serde_json::{Map, Value};

use crate::application::ports::sealer::Check;
use crate::application::ports::settings::{SafetySetting, Sealed};
use crate::application::ports::SettingsStore;

pub const AI_SHARE_MODE_KEY: &str = SafetySetting::ShareMode.key();
pub const AI_SHARE_PATTERNS_KEY: &str = SafetySetting::SharePatterns.key();
pub const DEFAULT_PATTERNS: &str = "email, phone, token, password, passwd, secret, credential, apikey, api_key, authorization, cookie, iban, card*";
pub const MASK: &str = "•••";

/// The tools whose answer can hold job data.
pub const JOB_DATA_TOOLS: &[&str] = &[
    "list_jobs",
    "inspect_job",
    "list_failed_jobs",
    "get_flow",
    "list_schedulers",
    "get_job_logs",
    "list_failure_causes",
];

/// The keys whose value is job data (payload, result, progress, a
/// scheduler's job template).
const PAYLOAD_KEYS: &[&str] = &["data", "returnValue", "returnvalue", "progress", "template"];

const WITHHELD_NOTE: &str = "Ranch shares job names, states and errors only with agents: payloads, return values, progress, templates and logs are not sent.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SharePolicy {
    Everything,
    Mask(Vec<String>),
    NamesOnly,
}

/// The policy in force. An unreadable setting shares names only: a broken
/// database must never widen what agents see.
/// A mode the app did not seal shares names only, too.
pub async fn share_policy(settings: &dyn SettingsStore) -> SharePolicy {
    let mode = match settings.safety(SafetySetting::ShareMode).await {
        Ok(Sealed {
            check: Check::Invalid,
            ..
        })
        | Err(_) => return SharePolicy::NamesOnly,
        Ok(Sealed { value, .. }) => value,
    };
    match mode.as_deref() {
        Some("everything") => SharePolicy::Everything,
        Some("names") => SharePolicy::NamesOnly,
        // Unset or `mask`: mask, with the default patterns when the user's
        // were not written by the app.
        _ => {
            let raw = match settings.safety(SafetySetting::SharePatterns).await {
                Ok(Sealed {
                    value: Some(raw),
                    check: Check::Valid | Check::Unverifiable,
                }) => raw,
                _ => DEFAULT_PATTERNS.to_string(),
            };
            SharePolicy::Mask(parse_patterns(&raw))
        }
    }
}

/// `"email, Card*"` → `["email", "card*"]` (lowercase, trimmed, no blanks).
pub fn parse_patterns(raw: &str) -> Vec<String> {
    raw.split([',', '\n'])
        .map(|p| p.trim().to_lowercase())
        .filter(|p| !p.is_empty())
        .collect()
}

/// A pattern without `*` matches any key that contains it (`email` masks
/// `customerEmail`); with `*` it is a glob over the whole key (`card*`
/// masks `cardNumber`, not `discard`). Case-insensitive.
pub fn key_matches(key: &str, patterns: &[String]) -> bool {
    let key = key.to_lowercase();
    patterns.iter().any(|p| {
        if p.contains('*') {
            glob(p, &key)
        } else {
            key.contains(p.as_str())
        }
    })
}

fn glob(pattern: &str, text: &str) -> bool {
    fn go(p: &[u8], t: &[u8]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some((b'*', rest)) => (0..=t.len()).any(|i| go(rest, &t[i..])),
            Some((c, rest)) => t.first() == Some(c) && go(rest, &t[1..]),
        }
    }
    go(pattern.as_bytes(), text.as_bytes())
}

/// Replace the value of every matching key, at any depth.
pub fn mask_value(value: &mut Value, patterns: &[String]) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map.iter_mut() {
                if key_matches(key, patterns) {
                    *inner = Value::String(MASK.to_string());
                } else {
                    mask_value(inner, patterns);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                mask_value(item, patterns);
            }
        }
        // A payload stored as a JSON string (a scheduler template): mask
        // inside it and keep it a string.
        Value::String(text) => {
            if let Ok(mut parsed @ (Value::Object(_) | Value::Array(_))) =
                serde_json::from_str::<Value>(text)
            {
                mask_value(&mut parsed, patterns);
                *text = parsed.to_string();
            }
        }
        _ => {}
    }
}

/// Apply the policy to a tool's JSON answer. Returns whether something was
/// withheld (names-only mode).
pub fn apply(policy: &SharePolicy, output: &mut Value) -> bool {
    match policy {
        SharePolicy::Everything => false,
        SharePolicy::Mask(patterns) => {
            visit_payloads(output, &mut |payload| mask_value(payload, patterns));
            false
        }
        SharePolicy::NamesOnly => {
            let mut withheld = false;
            visit_payloads(output, &mut |payload| {
                if !payload.is_null() {
                    *payload = Value::Null;
                    withheld = true;
                }
            });
            withheld | withhold_logs(output)
        }
    }
}

fn visit_payloads(value: &mut Value, f: &mut dyn FnMut(&mut Value)) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map.iter_mut() {
                if PAYLOAD_KEYS.contains(&key.as_str()) {
                    f(inner);
                } else {
                    visit_payloads(inner, f);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                visit_payloads(item, f);
            }
        }
        _ => {}
    }
}

/// Names only: job logs are free text written by the worker, often with the
/// payload in it. `get_job_logs` answers `{ lines, total, hasMore }`; `logs`
/// is kept for any answer that nests them under that name.
fn withhold_logs(value: &mut Value) -> bool {
    let Value::Object(map) = value else {
        return false;
    };
    let mut withheld = false;
    for key in ["lines", "logs"] {
        if let Some(Value::Array(lines)) = map.get_mut(key) {
            if !lines.is_empty() {
                lines.clear();
                withheld = true;
            }
        }
    }
    withheld
}

/// The text answer of a tool, with the policy applied. Not JSON (or
/// nothing to change): returned as is.
pub fn share_output(policy: &SharePolicy, text: &str) -> String {
    if *policy == SharePolicy::Everything {
        return text.to_string();
    }
    let Ok(mut value) = serde_json::from_str::<Value>(text) else {
        return text.to_string();
    };
    if apply(policy, &mut value) {
        if let Value::Object(map) = &mut value {
            map.insert("dataWithheld".into(), Value::String(WITHHELD_NOTE.into()));
        } else {
            let mut wrapped = Map::new();
            wrapped.insert("result".into(), value);
            wrapped.insert("dataWithheld".into(), Value::String(WITHHELD_NOTE.into()));
            value = Value::Object(wrapped);
        }
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn defaults() -> Vec<String> {
        parse_patterns(DEFAULT_PATTERNS)
    }

    #[test]
    fn patterns_match_names_not_values() {
        let p = defaults();
        assert!(key_matches("email", &p));
        assert!(key_matches("customerEmail", &p));
        assert!(key_matches("PHONE_NUMBER", &p));
        assert!(key_matches("accessToken", &p));
        assert!(key_matches("cardNumber", &p));
        assert!(key_matches("card", &p));
        assert!(!key_matches("discard", &p), "card* is a prefix glob");
        assert!(!key_matches("userId", &p));
        assert!(!key_matches("amount", &p));
        assert!(key_matches("x_secret_y", &parse_patterns("*secret*")));
        assert!(key_matches("api_key", &parse_patterns("*_key")));
        assert!(!key_matches("keyring", &parse_patterns("*_key")));
    }

    #[test]
    fn masking_reaches_every_depth_and_json_strings() {
        let mut v = json!({
            "jobs": [{
                "name": "send",
                "failedReason": "SMTP refused",
                "data": {
                    "to": { "email": "a@b.c", "name": "Ann" },
                    "items": [{ "cardNumber": "4111", "sku": "x" }],
                    "auth": { "accessToken": "t0k" }
                },
                "returnValue": { "password": "hunter2", "ok": true }
            }],
            "schedulers": [{ "template": "{\"iban\":\"FR76\",\"plan\":\"pro\"}" }]
        });
        apply(&SharePolicy::Mask(defaults()), &mut v);
        let text = v.to_string();
        for secret in ["a@b.c", "4111", "t0k", "hunter2", "FR76"] {
            assert!(!text.contains(secret), "{secret} leaked: {text}");
        }
        for kept in ["Ann", "sku", "SMTP refused", "pro", "send"] {
            assert!(text.contains(kept), "{kept} lost: {text}");
        }
        assert_eq!(v["jobs"][0]["data"]["to"]["email"], MASK);
    }

    #[test]
    fn names_only_keeps_names_states_and_errors() {
        let text = share_output(
            &SharePolicy::NamesOnly,
            &json!({
                "jobId": "1", "name": "send", "state": "failed",
                "failedReason": "boom", "data": { "note": "private" },
                "progress": 50, "returnValue": null
            })
            .to_string(),
        );
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["data"], Value::Null);
        assert_eq!(v["progress"], Value::Null);
        assert_eq!(v["failedReason"], "boom");
        assert_eq!(v["name"], "send");
        assert!(v["dataWithheld"].is_string());
        assert!(!text.contains("private"));
    }

    #[test]
    fn names_only_withholds_the_lines_of_get_job_logs() {
        // The exact shape of `JobLogsOutput`.
        let answer = crate::contract::jobs::JobLogsOutput {
            lines: vec!["sending to a@b.c".into(), "token=abc".into()],
            total: 2,
            has_more: false,
        };
        let text = share_output(
            &SharePolicy::NamesOnly,
            &serde_json::to_string(&answer).unwrap(),
        );
        assert!(!text.contains("a@b.c"), "{text}");
        assert!(!text.contains("token=abc"), "{text}");
        assert!(text.contains("dataWithheld"));
    }

    #[test]
    fn names_only_withholds_logs() {
        let text = share_output(
            &SharePolicy::NamesOnly,
            &json!({ "logs": ["sent to a@b.c"], "count": 1 }).to_string(),
        );
        assert!(!text.contains("a@b.c"));
        assert!(text.contains("dataWithheld"));
    }

    #[test]
    fn everything_and_non_json_pass_through() {
        let raw = json!({ "data": { "email": "a@b.c" } }).to_string();
        assert_eq!(share_output(&SharePolicy::Everything, &raw), raw);
        assert_eq!(
            share_output(&SharePolicy::Mask(defaults()), "plain"),
            "plain"
        );
    }

    #[test]
    fn metrics_like_data_outside_payload_keys_is_untouched() {
        let mut v = json!({ "metrics": { "points": [1, 2, 3] }, "name": "q" });
        assert!(!apply(&SharePolicy::NamesOnly, &mut v));
        assert_eq!(v["metrics"]["points"], json!([1, 2, 3]));
    }
}
