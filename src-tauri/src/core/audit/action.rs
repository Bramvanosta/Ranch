//! Audit action enum.
//!
//! Each variant represents one type of destructive operation Ranch can
//! perform. Stored in the audit log's `action` column as a discriminator
//! and serialized to camelCase on the wire so the frontend can match
//! it against a TypeScript union.
//!
//! **Adding variants** is the norm — every new destructive command
//! (retry/remove/promote/pause/...) appends a variant here. **Never
//! remove a variant**: the audit log is append-only and old entries
//! reference these strings.

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AuditAction {
    RetryJob,
    RemoveJob,
    PromoteJob,
    PauseQueue,
    ResumeQueue,
    PauseScheduler,
    ResumeScheduler,
    UnlockConnection,
    LockConnection,
    /// A job enqueued from Ranch (UI or agent).
    AddJob,
    /// queue-level maintenance.
    DrainQueue,
    CleanQueue,
    RetryAllFailed,
    /// A job scheduler created / changed, or deleted.
    UpsertScheduler,
    RemoveScheduler,
}

impl AuditAction {
    /// String discriminator used in the SQL `action` column. Matches the
    /// camelCase wire shape one-to-one so list/export filters can compare
    /// the column to a wire value without translation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RetryJob => "retryJob",
            Self::RemoveJob => "removeJob",
            Self::PromoteJob => "promoteJob",
            Self::PauseQueue => "pauseQueue",
            Self::ResumeQueue => "resumeQueue",
            Self::PauseScheduler => "pauseScheduler",
            Self::ResumeScheduler => "resumeScheduler",
            Self::UnlockConnection => "unlockConnection",
            Self::LockConnection => "lockConnection",
            Self::AddJob => "addJob",
            Self::DrainQueue => "drainQueue",
            Self::CleanQueue => "cleanQueue",
            Self::RetryAllFailed => "retryAllFailed",
            Self::UpsertScheduler => "upsertScheduler",
            Self::RemoveScheduler => "removeScheduler",
        }
    }

    /// Inverse of [`as_str`] — parse a stored discriminator back to the
    /// enum. Returns `None` for unknown values (forward-compat with
    /// future variants written by a newer Ranch build).
    ///
    /// Named `try_from_str` (not `from_str`) so it doesn't shadow the
    /// `std::str::FromStr` trait method (clippy::should_implement_trait).
    pub fn try_from_str(s: &str) -> Option<Self> {
        Some(match s {
            "retryJob" => Self::RetryJob,
            "removeJob" => Self::RemoveJob,
            "promoteJob" => Self::PromoteJob,
            "pauseQueue" => Self::PauseQueue,
            "resumeQueue" => Self::ResumeQueue,
            "pauseScheduler" => Self::PauseScheduler,
            "resumeScheduler" => Self::ResumeScheduler,
            "unlockConnection" => Self::UnlockConnection,
            "lockConnection" => Self::LockConnection,
            "addJob" => Self::AddJob,
            "drainQueue" => Self::DrainQueue,
            "cleanQueue" => Self::CleanQueue,
            "retryAllFailed" => Self::RetryAllFailed,
            "upsertScheduler" => Self::UpsertScheduler,
            "removeScheduler" => Self::RemoveScheduler,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_to_camel_case() {
        let json = serde_json::to_string(&AuditAction::RetryJob).unwrap();
        assert_eq!(json, "\"retryJob\"");
        let json = serde_json::to_string(&AuditAction::PauseScheduler).unwrap();
        assert_eq!(json, "\"pauseScheduler\"");
    }

    #[test]
    fn as_str_matches_camel_case() {
        for v in [
            AuditAction::RetryJob,
            AuditAction::RemoveJob,
            AuditAction::PromoteJob,
            AuditAction::PauseQueue,
            AuditAction::ResumeQueue,
            AuditAction::PauseScheduler,
            AuditAction::ResumeScheduler,
            AuditAction::UnlockConnection,
            AuditAction::LockConnection,
            AuditAction::AddJob,
            AuditAction::DrainQueue,
            AuditAction::CleanQueue,
            AuditAction::RetryAllFailed,
            AuditAction::UpsertScheduler,
            AuditAction::RemoveScheduler,
        ] {
            let json = serde_json::to_string(&v).unwrap();
            let unquoted = json.trim_matches('"');
            assert_eq!(unquoted, v.as_str(), "mismatch for {v:?}");
            assert_eq!(AuditAction::try_from_str(v.as_str()), Some(v));
        }
    }

    #[test]
    fn try_from_str_unknown_returns_none() {
        assert_eq!(AuditAction::try_from_str("retryAllTheThings"), None);
    }
}
