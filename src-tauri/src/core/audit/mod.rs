//! Audit log funnel.
//!
//! THE ONLY MODULE that writes to the `audit_log_entries` SQLite table.
//! The [`record`] function (re-exported from `writer::record`) is the
//! single public surface; the `writer` submodule itself is `pub(super)`
//! so direct SQL on the audit table from feature code is impossible.
//!
//! ## Architectural invariants
//!
//! 1. **Append-only.** No delete path exists. No Tauri command exposes
//!    deletion; no MCP tool exposes deletion. Erasing audit data
//!    requires explicit filesystem action on `ranch.db`.
//! 2. **Synchronous flush.** Every write commits with `PRAGMA
//!    synchronous = FULL` per-transaction. A write the user sees
//!    confirmed has been fsync'd to disk before the UI updates.
//! 3. **Module visibility = bypass prevention.** The `writer` submodule
//!    is `pub(super)`. Feature code calls `core::audit::record(...)`
//!    or it doesn't write — there is no third option.
//!
//! ## Adding new variants
//!
//! - Add a variant to [`AuditAction`] for each new destructive command
//!   class. Never remove a variant; the audit log is append-only.
//! - Add a variant to [`AuditTarget`] for each new target shape.
//! - Existing reads continue to work — the stored discriminator
//!   strings are versioned forever.

mod action;
mod domain;
mod target;
mod writer;

pub use action::AuditAction;
pub use domain::{AuditEntry, AuditLogEntry, AuditOutcome, AuditSource};
pub use target::AuditTarget;
pub use writer::record;
