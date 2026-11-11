//! The app crate's own logic around the ports: the policy around
//! agents (`mcp`), cause notes, the MCP installer, the macOS app bundle.
//!
//! The adapters are elsewhere: the database, its migrations and the audit
//! writer in `adapters::sqlite`, the connection actors and what BullMQ keeps
//! in Redis in `adapters::bullmq_redis`.

pub mod app_bundle;
pub mod cause_notes;
pub mod join;
pub mod mcp;
pub mod mcp_installer;
