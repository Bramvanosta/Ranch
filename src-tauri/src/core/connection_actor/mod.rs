//! per-connection Tokio actor.
//!
//! Each saved Connection that any caller (Tauri command OR MCP tool)
//! wants to operate on owns ONE Tokio task that holds the `redis::Client`,
//! cancellation token, and lock state. The actor receives
//! messages via a bounded `tokio::sync::mpsc::channel(64)`; concurrent
//! UI + MCP callers serialize through the mailbox so per-connection
//! state mutations are race-free by construction.
//!
//! ## Lifecycle
//!
//!   - **Spawn**: lazy — first call to `ConnectionRegistry::ping(id, cfg)`
//!     for an unknown id spins up the actor and inserts a handle.
//!   - **Reuse**: subsequent calls find the handle and forward messages.
//!   - **Shutdown**: explicit via `ConnectionRegistry::shutdown(id)`
//!     (called from `remove_connection`) OR registry drop at app exit.
//!
//! ## Cancellation contract
//!
//! Every Redis I/O the actor runs is wrapped in `tokio::select! { res =
//! op => ..., _ = token.cancelled() => Cancelled }`. On `Shutdown` the
//! token fires; the actor's loop drains pending messages with `Cancelled`
//! replies, then exits. NFR: in-flight ops cancel within 1 second.
//!
//! ## What the actor handles
//!
//! The actor scaffold started from a single `Ping` message (the
//! `ping_connection` reachability path); the `ActorCommand` enum extends it:
//!
//!   - `ListQueues`, `GetCounters` — BullMQ-aware messages, integrating
//!     `core::bullmq_schema`.
//!   - `ListJobs`, `InspectJob`.
//!   - `audit::record(...)` calls inside destructive handlers.
//!   - `lock_state: ReadOnlyState` + lock checks inside the actor.

mod actor;
mod handle;
mod registry;

pub use actor::{AuditAttribution, ConnectionConfig, EventSink};
pub use handle::{ActorError, ConnectionHandle};
pub use registry::ConnectionRegistry;
// re-export so feature code can use the same path as the
// other actor-adjacent types.
pub use crate::core::safety_lock::ReadOnlyState;
