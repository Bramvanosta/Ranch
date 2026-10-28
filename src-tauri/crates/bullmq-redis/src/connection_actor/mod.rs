//! One Tokio task per connection in use.
//!
//! Each saved connection that a caller (a Tauri command, an HTTP route or an
//! MCP tool, through the BullMQ backend) operates on owns ONE task that holds
//! its persistent Redis link, the SSH tunnel, the cancellation token and the
//! live lock state. The actor receives messages via a bounded
//! `tokio::sync::mpsc::channel(64)`. Reads run concurrently on copies of its
//! state; writes and lock changes run in its loop, one at a time, so
//! per-connection state changes are race-free by construction.
//!
//! ## Lifecycle
//!
//!   - **Spawn**: lazy; the first call of `ConnectionRegistry` for an
//!     unknown id spins up the actor and inserts a handle.
//!   - **Reuse**: subsequent calls find the handle and forward messages.
//!   - **Shutdown**: explicit via `ConnectionRegistry::shutdown(id)`
//!     (when a connection is removed or edited) OR registry drop at exit.
//!
//! ## Cancellation contract
//!
//! Every Redis I/O the actor runs is wrapped in `tokio::select! { res =
//! op => ..., _ = token.cancelled() => Cancelled }`. On `Shutdown` the
//! token fires; the actor's loop drains pending messages with `Cancelled`
//! replies, then exits. In-flight operations cancel within one second.
//!
//! ## What the actor handles
//!
//! One `ActorCommand` per operation: reachability, reads (queues, jobs,
//! metrics, workers, flows, schedulers, events) decoded through
//! `crate::bullmq_schema`, and writes, each run through the write pipeline
//! (`ranch_application::writes`): authorized against the connection's live
//! `lock_state` or an approval, then recorded in the audit log before it
//! replies.
//!
//! ## Layout
//!
//!   - `actor`: config, command enum, state and the run loop.
//!   - `link`: the Redis connection, SSH tunnel, ping and key resolution.
//!   - `discovery`: the queue registry. `follow`: the event follower.
//!   - `reads/`, `writes/`: the handlers, one file per family; every
//!     write passes `writes::authorize` first.

mod actor;
mod discovery;
mod follow;
mod handle;
mod link;
mod reads;
mod registry;
mod writes;

pub use actor::ConnectionConfig;
pub use follow::EventSink;
pub use handle::{ActorError, ConnectionHandle};
pub use registry::ConnectionRegistry;
// re-export so feature code can use the same path as the
// other actor-adjacent types.
pub use ranch_domain::lock::ReadOnlyState;
