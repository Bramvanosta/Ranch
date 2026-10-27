//! Domain core — IO-free domain logic + IO adapters.
//!
//! Module layout follows architecture § Project Structure & Boundaries:
//!   - `errors`   — typed `RanchError` enum used across the backend
//!   - `db`       — SqlitePool + migrations
//!   - `keychain` — OS keychain abstraction (trait + real + in-memory impls)
//!
//! `audit` is the synchronous-flush write funnel; `bullmq_schema` and
//! `connection_actor` build on the data-layer foundation.

pub mod audit;
pub mod bullmq_schema;
pub mod cause_notes;
pub mod connection_actor;
pub mod db;
pub mod errors;
pub mod events;
pub mod join;
pub mod keychain;
pub mod mcp;
pub mod mcp_installer;
pub mod redis_info;
pub mod safety_lock;
pub mod ssh_config;
pub mod ssh_tunnel;
pub mod tls;
