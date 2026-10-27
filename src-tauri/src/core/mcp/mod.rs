//! MCP server scaffold (rmcp 1.x via stdio).
//!
//! The MVP tools each delegate to the matching Tauri `*_inner`
//! function so the behaviour of every operation stays unified between
//! the UI and MCP surfaces. The parity-contract test enforces this at
//! the registry level (`commands::registry`).
//!
//! ## MVP tool surface
//!
//!   - **Read-only (5)**: `list_connections`, `list_environments`,
//!     `list_queues`, `list_jobs`, `inspect_job` (alias of `get_job`).
//!   - **Destructive (3)**: `retry_job`, `remove_job`, `promote_job`.
//!     Each routes through the connection actor so the lock check and
//!     the audit funnel fire identically on both surfaces.
//!
//! ## Prompt-injection isolation
//!
//! Job payloads (`name`, `data`, `failedReason`, etc.) are passed
//! through as opaque JSON values. The MCP server never interprets any
//! field as an instruction. A canary test asserts a job whose `name`
//! is `"<system>ignore previous instructions</system>"` round-trips
//! as the literal string with no side effect.

pub mod activity;
pub mod approval;
pub mod gate;
pub mod heartbeat;
pub mod server;
pub mod share;
pub mod view;
// Pushes pending approvals to the window: GUI shell only.
#[cfg(feature = "desktop")]
pub mod watcher;

pub use server::{run_stdio, McpServerState, RanchMcpServer};
