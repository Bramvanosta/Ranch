//! Schema discriminator + decoder trait + error type.
//!
//! `BullmqSchema` carries the version Ranch detected for a connection.
//! `JobDecoder` is the per-version trait that turns raw HGETALL fields
//! into a typed [`Job`]. `SchemaError` is the typed error variant exposed
//! across the boundary (never panic, never produce malformed state
//! — return one of these variants instead).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use super::domain::{Job, JobRef};

/// Detected BullMQ schema for a connection. Resolved once at connection-
/// open time (the `ConnectionActor`) by reading the queue meta
/// hash; cached on the actor for the connection's lifetime. Re-detected
/// on every app launch (we don't persist it in SQLite — version of the
/// remote BullMQ deployment is the authoritative source).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BullmqSchema {
    V5,
    V6,
    /// The remote BullMQ deployment's `version` field doesn't match a
    /// supported major. The frontend surfaces this verbatim so the user
    /// can file a compatibility issue or upgrade Ranch.
    Unknown {
        detected_version: Option<String>,
    },
}

#[derive(Debug, Clone, Error, Serialize, Deserialize, Type, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SchemaError {
    /// The decoder was asked to act on a connection whose BullMQ version
    /// Ranch doesn't recognize. Carry the raw `version` field so the UI
    /// can show the actual mismatch.
    #[error("BullMQ schema not recognized (detected version: {detected_version:?})")]
    SchemaNotRecognized { detected_version: Option<String> },

    /// A required job-hash field was absent from the HGETALL response.
    #[error("Required job field missing: {field}")]
    MissingField { field: String },

    /// A field is present but encoded in a way the decoder couldn't
    /// interpret (non-utf8, non-numeric where a number was expected, etc.).
    #[error("Job field `{field}` has invalid encoding: {reason}")]
    InvalidField { field: String, reason: String },

    /// A field that BullMQ stores as a JSON string didn't parse.
    #[error("Job payload JSON parse failed for field `{field}`: {reason}")]
    InvalidJson { field: String, reason: String },
}

/// Detect the BullMQ schema version from the `bull:{queue}:meta` hash.
/// BullMQ writes a `version` field that we parse to pick the right decoder.
///
/// ## Real-world version formats
///
/// Verified against running BullMQ instances:
///   - `"bullmq:5.76.6"` — BullMQ v5.x (most common in the wild; the
///     library prepends its package name).
///   - `"5.7.0"` / `"6.1.0"` — bare semver (used by some BullMQ ports
///     and by our test fixtures).
///   - `"v5.1"` — leading `v` prefix (defensive parse — never seen in
///     real data, but cheap to support).
///
/// The parser strips a leading `bullmq:` prefix, then a leading `v`,
/// then takes the first dot-separated segment as the major version.
/// Missing field → `Unknown { None }`. Unparseable → `Unknown { Some(raw) }`.
pub fn detect_schema(meta_hash: &HashMap<String, String>) -> BullmqSchema {
    let Some(raw) = meta_hash.get("version").map(|v| v.trim().to_string()) else {
        return BullmqSchema::Unknown {
            detected_version: None,
        };
    };
    let normalized = raw.trim_start_matches("bullmq:").trim_start_matches('v');
    let major = normalized
        .split('.')
        .next()
        .unwrap_or("")
        .parse::<u32>()
        .ok();
    match major {
        Some(5) => BullmqSchema::V5,
        Some(6) => BullmqSchema::V6,
        _ => BullmqSchema::Unknown {
            detected_version: Some(raw),
        },
    }
}

/// Per-version decoder trait. Each `vN.rs` ships a zero-sized
/// implementation. Feature code holds `Box<dyn JobDecoder>` or one of the
/// concrete `Copy` types; the dispatch enum [`KeyVariant`](super::keys::KeyVariant)
/// pattern can be replicated for decoders if hot-path dispatch becomes a
/// concern.
pub trait JobDecoder {
    fn decode_job(
        &self,
        jref: JobRef,
        fields: HashMap<String, Vec<u8>>,
    ) -> Result<Job, SchemaError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(version: Option<&str>) -> HashMap<String, String> {
        let mut m = HashMap::new();
        if let Some(v) = version {
            m.insert("version".into(), v.into());
        }
        m
    }

    #[test]
    fn detect_v5() {
        assert_eq!(detect_schema(&meta(Some("5.0.0"))), BullmqSchema::V5);
        assert_eq!(detect_schema(&meta(Some("5.40.7"))), BullmqSchema::V5);
        assert_eq!(detect_schema(&meta(Some("v5.1"))), BullmqSchema::V5);
    }

    #[test]
    fn detect_v5_with_bullmq_prefix_real_world() {
        // BullMQ v5.x in the wild stores the version as "bullmq:5.76.6" —
        // verified against a running redis:7-alpine + bullmq@5.76.6.
        // Without the prefix-strip the parser fails and returns Unknown.
        assert_eq!(
            detect_schema(&meta(Some("bullmq:5.76.6"))),
            BullmqSchema::V5
        );
        assert_eq!(detect_schema(&meta(Some("bullmq:5.0.0"))), BullmqSchema::V5);
    }

    #[test]
    fn detect_v6() {
        assert_eq!(detect_schema(&meta(Some("6.0.0"))), BullmqSchema::V6);
        assert_eq!(detect_schema(&meta(Some("6.2.1"))), BullmqSchema::V6);
        assert_eq!(detect_schema(&meta(Some("v6"))), BullmqSchema::V6);
    }

    #[test]
    fn detect_v6_with_bullmq_prefix() {
        // Defensive: BullMQ v6 will likely keep the same prefix convention.
        assert_eq!(detect_schema(&meta(Some("bullmq:6.0.0"))), BullmqSchema::V6);
        assert_eq!(detect_schema(&meta(Some("bullmq:6.4.2"))), BullmqSchema::V6);
    }

    #[test]
    fn detect_unknown_for_future_major() {
        assert_eq!(
            detect_schema(&meta(Some("7.0.0"))),
            BullmqSchema::Unknown {
                detected_version: Some("7.0.0".into())
            }
        );
    }

    #[test]
    fn detect_unknown_for_legacy_major() {
        assert_eq!(
            detect_schema(&meta(Some("4.12.0"))),
            BullmqSchema::Unknown {
                detected_version: Some("4.12.0".into())
            }
        );
    }

    #[test]
    fn detect_unknown_when_version_missing() {
        assert_eq!(
            detect_schema(&meta(None)),
            BullmqSchema::Unknown {
                detected_version: None
            }
        );
    }

    #[test]
    fn detect_unknown_for_garbage_version() {
        assert_eq!(
            detect_schema(&meta(Some("not-a-version"))),
            BullmqSchema::Unknown {
                detected_version: Some("not-a-version".into())
            }
        );
    }
}
