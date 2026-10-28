//! Audit log funnel.
//!
//! THE ONLY MODULE that writes to the `audit_log_entries` SQLite table.
//! The [`record`] function (re-exported from `writer::record`) is the
//! single public surface; the `writer` submodule itself is private
//! so direct SQL on the audit table from feature code is impossible. What
//! an entry holds is the domain's (`domain::audit`).
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
//!    is private. A writer calls [`record`] (the app crate's
//!    `adapters::sqlite::audit::record`), directly or through the write
//!    pipeline's `AuditLog` port, or it doesn't write; there is no third
//!    option.

mod writer;

pub use writer::record;
