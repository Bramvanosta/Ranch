//! What the audit log records: who asked ([`Caller`], [`AuditSource`]),
//! what was done to what ([`AuditAction`], [`AuditTarget`]), how it ended
//! ([`AuditOutcome`]), the entry written ([`AuditEntry`]) and the row read
//! back ([`AuditLogEntry`]). The writer, the one way into the table, is
//! the SQLite crate's (`ranch_sqlite::audit`).
//!
//! ## Adding new variants
//!
//! - Add a variant to [`AuditAction`] for each new destructive command
//!   class. Never remove a variant; the audit log is append-only.
//! - Add a variant to [`AuditTarget`] for each new target shape.
//! - Existing reads continue to work: the stored discriminator
//!   strings are versioned forever.

mod action;
mod caller;
mod entry;
mod target;

pub use action::AuditAction;
pub use caller::{ApprovalGrant, Caller};
pub use entry::{AuditEntry, AuditLogEntry, AuditOutcome, AuditSource};
pub use target::AuditTarget;
