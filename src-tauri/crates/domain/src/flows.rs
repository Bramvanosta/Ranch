//! Flows: parent and child jobs, as a flat tree and as the roots a queue
//! holds.

use super::jobs::{JobRef, JobState};

/// One job of a flow. The tree is flat (`parent_key` links the nodes) so
/// it crosses the IPC boundary without recursive types and feeds a graph
/// layout directly.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowNode {
    /// `{prefix}:{queue}:{id}`, unique within the flow.
    pub key: String,
    pub job_ref: JobRef,
    /// `None` for the root.
    pub parent_key: Option<String>,
    pub name: String,
    pub state: JobState,
    pub timestamp: Option<i64>,
    /// `finishedOn − processedOn`, when both are known.
    pub duration_ms: Option<i64>,
    pub failed_reason: Option<String>,
    /// The job hash no longer exists (removed by `removeOnComplete`, a clean…):
    /// the parent still references it.
    pub missing: bool,
    /// What a finished job returned (raw JSON), if anything.
    pub return_value: Option<String>,
    /// This job failed for good and its parent waits for it
    /// forever (no `failParentOnFailure` / `ignoreDependencyOnFailure`…).
    pub blocks_parent: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlowTree {
    /// Root first, then breadth-first.
    pub nodes: Vec<FlowNode>,
    pub root_key: String,
    /// The job the flow was asked for.
    pub focus_key: String,
    /// The flow holds more than [`MAX_NODES`] jobs; the rest is not shown.
    pub truncated: bool,
    /// A parent waits on a child that failed for good.
    pub blocked: bool,
}

/// A flow root found in a queue (the queue's Flows tab).
#[derive(Debug, Clone, PartialEq)]
pub struct FlowRoot {
    pub job_ref: JobRef,
    pub name: String,
    pub state: JobState,
    pub timestamp: Option<i64>,
    /// Direct children, by what became of them.
    pub children_total: u64,
    pub children_pending: u64,
    pub children_completed: u64,
    pub children_failed: u64,
    /// The root waits on a child that failed for good.
    pub blocked: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlowRoots {
    /// Newest first.
    pub roots: Vec<FlowRoot>,
    /// Jobs looked at.
    pub scanned: u64,
    /// The queue holds more finished jobs than discovery looks at: older
    /// flows are not listed.
    pub capped: bool,
}

impl FlowTree {
    /// A job on its own is not a flow.
    pub fn is_flow(&self) -> bool {
        self.nodes.len() > 1
    }
}
