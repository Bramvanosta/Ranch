//! What happened on a connection's queues, read from their event streams.

use super::queues::QueueId;

#[derive(Debug, Clone, PartialEq)]
pub struct QueueEvent {
    pub queue_id: QueueId,
    pub queue_name: String,
    /// The stream id (`<ms>-<seq>`), unique within the queue.
    pub stream_id: String,
    pub at_ms: f64,
    /// `added`, `active`, `completed`, `failed`, `delayed`, `stalled`,
    /// `removed`, or `retried` (BullMQ's `waiting` after a failure).
    pub event: String,
    pub job_id: Option<String>,
    pub job_name: Option<String>,
    pub failed_reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueueEvents {
    /// Newest first.
    pub events: Vec<QueueEvent>,
    /// Where to read from next time, one per queue read.
    pub cursors: Vec<EventCursor>,
    /// More happened than was read: the newest were kept.
    pub sampled: bool,
}

/// What happened since the last read: where each queue's last read
/// stopped (a queue without a cursor is read from its last few events),
/// and how many events at most (default 100).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventsQuery {
    pub cursors: Vec<EventCursor>,
    pub limit: Option<u32>,
}

/// Which queues of a followed connection moved since the last notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEventsNotice {
    pub connection_id: String,
    pub queues: Vec<QueueEventsSeen>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEventsSeen {
    pub queue_id: QueueId,
    /// Event names seen (`added`, `completed`, `failed`, …), each once.
    pub events: Vec<String>,
    /// Entries read, capped by the read's count.
    pub count: u32,
}

/// Where the last read of a queue stopped (a stream id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventCursor {
    pub queue_id: QueueId,
    pub last_id: String,
}
