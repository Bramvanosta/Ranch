//! The BullMQ schema Ranch detects on a connection and the errors a job
//! decoder returns, as the bindings describe them.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::adapters::bullmq_redis::bullmq_schema as bullmq;

/// Detected BullMQ schema for a connection. Resolved once at connection-
/// open time (the `ConnectionActor`) by reading the queue meta
/// hash; cached on the actor for the connection's lifetime. Re-detected
/// on every app launch (we don't persist it in SQLite: the version of the
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

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SchemaError {
    /// The decoder was asked to act on a connection whose BullMQ version
    /// Ranch doesn't recognize. Carry the raw `version` field so the UI
    /// can show the actual mismatch.
    SchemaNotRecognized { detected_version: Option<String> },

    /// A required job-hash field was absent from the HGETALL response.
    MissingField { field: String },

    /// A field is present but encoded in a way the decoder couldn't
    /// interpret (non-utf8, non-numeric where a number was expected, etc.).
    InvalidField { field: String, reason: String },

    /// A field that BullMQ stores as a JSON string didn't parse.
    InvalidJson { field: String, reason: String },
}

impl From<bullmq::BullmqSchema> for BullmqSchema {
    fn from(v: bullmq::BullmqSchema) -> Self {
        match v {
            bullmq::BullmqSchema::V5 => Self::V5,
            bullmq::BullmqSchema::V6 => Self::V6,
            bullmq::BullmqSchema::Unknown { detected_version } => {
                Self::Unknown { detected_version }
            }
        }
    }
}

impl From<bullmq::SchemaError> for SchemaError {
    fn from(v: bullmq::SchemaError) -> Self {
        match v {
            bullmq::SchemaError::SchemaNotRecognized { detected_version } => {
                Self::SchemaNotRecognized { detected_version }
            }
            bullmq::SchemaError::MissingField { field } => Self::MissingField { field },
            bullmq::SchemaError::InvalidField { field, reason } => {
                Self::InvalidField { field, reason }
            }
            bullmq::SchemaError::InvalidJson { field, reason } => {
                Self::InvalidJson { field, reason }
            }
        }
    }
}
