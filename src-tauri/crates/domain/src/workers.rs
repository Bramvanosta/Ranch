//! The workers consuming a queue, as the server reports them.

use std::collections::BTreeMap;

use super::queues::QueueId;

/// One BullMQ worker = one named Redis connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueWorker {
    /// Redis client id (`id=`), unique per connection on that server.
    pub id: String,
    /// The worker's own name (`new Worker(q, fn, { name })`), if it has one.
    pub name: Option<String>,
    /// `ip:port` of the worker process as Redis sees it.
    pub addr: String,
    /// Seconds since the connection was opened.
    pub age_secs: u64,
    /// Seconds since its last command. A healthy idle worker sits in a
    /// blocking pop, so this stays under its `drainDelay` (5 s by default).
    pub idle_secs: u64,
    /// Last command (`cmd=`), e.g. `bzpopmin`.
    pub last_command: Option<String>,
    /// Client library as reported by Redis ≥ 7.2 (`lib-name` / `lib-ver`).
    pub library: Option<String>,
    /// Every `key=value` of the `CLIENT LIST` line, for the detail sheet.
    pub raw: BTreeMap<String, String>,
}

/// Result of `list_workers`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueWorkers {
    /// `false` = this Redis refuses `CLIENT LIST`; `workers` is empty and
    /// `unsupported_reason` carries the server's message.
    pub supported: bool,
    pub unsupported_reason: Option<String>,
    pub workers: Vec<QueueWorker>,
}

impl QueueWorkers {
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self {
            supported: false,
            unsupported_reason: Some(reason.into()),
            workers: Vec::new(),
        }
    }
}

/// How many workers one queue has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueWorkerCount {
    pub queue_id: QueueId,
    pub workers: u32,
}

/// Result of `count_queue_workers`: one `CLIENT LIST` for many queues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueWorkerCounts {
    /// `false` = this Redis refuses `CLIENT LIST`: `counts` is empty, and no
    /// queue may be called worker-less from it.
    pub supported: bool,
    pub counts: Vec<QueueWorkerCount>,
}
