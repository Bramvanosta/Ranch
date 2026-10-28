//! The policy around agents: the write gate and approvals, job-data
//! sharing, the activity log, the app's liveness. The MCP server that
//! applies it is a shell, in `shells::mcp`.

pub mod activity;
pub mod approval;
pub mod gate;
pub mod heartbeat;
pub mod share;
// Pushes pending approvals to the window: GUI shell only.
#[cfg(feature = "desktop")]
pub mod watcher;
