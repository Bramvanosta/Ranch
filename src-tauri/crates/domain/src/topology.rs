//! How the queues of a connection link up through flows, how fast jobs
//! are added, and how old the oldest waiting job is.

use super::queues::QueueId;

#[derive(Debug, Clone, PartialEq)]
pub struct FlowLink {
    pub parent: QueueId,
    pub child: QueueId,
    /// Child jobs seen on the link (among the newest parents).
    pub jobs: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueueFlowStats {
    pub queue_id: QueueId,
    /// `added` events per minute over the last hour.
    pub added_per_minute: f64,
    /// The stream did not reach back an hour: a lower bound.
    pub added_capped: bool,
    /// When the oldest waiting job was added (ms); `None` = nothing waits.
    pub oldest_waiting_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Topology {
    pub links: Vec<FlowLink>,
    pub queues: Vec<QueueFlowStats>,
}
