//! Redis-key builders for BullMQ.
//!
//! The [`Keys`] trait is the only place in Ranch that knows BullMQ's key
//! conventions. Feature code (`commands/queues.rs`, etc.) calls these
//! methods to obtain a Redis key — never builds one via `format!("bull:…")`.
//!
//! Per-version implementations live in `v5.rs` and `v6.rs`. Each impl is a
//! zero-sized type (`Copy`) so callers can hold one inline without
//! allocating; the [`KeyVariant`] dispatch enum lets a single value carry
//! "which version's keys to use" without dynamic dispatch.

use super::domain::{JobRef, QueueRef};

/// Public trait — every Redis key Ranch reads is constructed via these
/// methods. Implementations are version-pinned (one per supported BullMQ
/// minor); the trait surface is identical so feature code can hold a
/// version-agnostic [`KeyVariant`] and route at the call site.
pub trait Keys {
    fn wait(&self, q: &QueueRef) -> String;
    fn active(&self, q: &QueueRef) -> String;
    fn completed(&self, q: &QueueRef) -> String;
    fn failed(&self, q: &QueueRef) -> String;
    fn delayed(&self, q: &QueueRef) -> String;
    fn prioritized(&self, q: &QueueRef) -> String;
    fn waiting_children(&self, q: &QueueRef) -> String;
    fn stalled(&self, q: &QueueRef) -> String;
    fn job_hash(&self, jref: &JobRef) -> String;
    fn meta(&self, q: &QueueRef) -> String;
    fn paused(&self, q: &QueueRef) -> String;
    fn repeat(&self, q: &QueueRef) -> String;
    fn logs(&self, jref: &JobRef) -> String;

    /// `{prefix}:{queue}` — what every key of the queue starts with. Same in
    /// every supported BullMQ version, so it derives from `meta`.
    fn queue_base(&self, q: &QueueRef) -> String {
        let meta = self.meta(q);
        meta.strip_suffix(":meta").unwrap_or(&meta).to_string()
    }

    /// The job id counter (`INCR`).
    fn id_counter(&self, q: &QueueRef) -> String {
        format!("{}:id", self.queue_base(q))
    }

    /// The events stream `QueueEvents` reads.
    fn events(&self, q: &QueueRef) -> String {
        format!("{}:events", self.queue_base(q))
    }

    /// The zset idle workers block on (`BZPOPMIN`): member `"0"`
    /// = a job is available, member `"1"` scored with the next delayed time.
    fn marker(&self, q: &QueueRef) -> String {
        format!("{}:marker", self.queue_base(q))
    }

    /// Counter that keeps FIFO order within one priority.
    fn priority_counter(&self, q: &QueueRef) -> String {
        format!("{}:pc", self.queue_base(q))
    }

    /// BullMQ native metrics: the meta hash (`count`, `prevTS`,
    /// `prevCount`) for `kind` = `completed` | `failed`. Same layout in every
    /// supported BullMQ version, so it derives from `meta`.
    fn metrics_meta(&self, q: &QueueRef, kind: &str) -> String {
        let meta = self.meta(q);
        let base = meta.strip_suffix(":meta").unwrap_or(&meta);
        format!("{base}:metrics:{kind}")
    }

    /// The per-minute data points list of [`Keys::metrics_meta`].
    fn metrics_data(&self, q: &QueueRef, kind: &str) -> String {
        format!("{}:data", self.metrics_meta(q, kind))
    }
}

/// Dispatch enum for "the right `Keys` for this connection's schema".
/// Stores nothing (each variant wraps a zero-sized type) — `Copy` lets
/// callers pass it cheaply through hot paths.
///
/// Future BullMQ minors with key-shape drift add a new variant here.
#[derive(Debug, Clone, Copy)]
pub enum KeyVariant {
    V5(super::v5::V5Keys),
    V6(super::v6::V6Keys),
}

impl Keys for KeyVariant {
    fn wait(&self, q: &QueueRef) -> String {
        match self {
            Self::V5(k) => k.wait(q),
            Self::V6(k) => k.wait(q),
        }
    }
    fn active(&self, q: &QueueRef) -> String {
        match self {
            Self::V5(k) => k.active(q),
            Self::V6(k) => k.active(q),
        }
    }
    fn completed(&self, q: &QueueRef) -> String {
        match self {
            Self::V5(k) => k.completed(q),
            Self::V6(k) => k.completed(q),
        }
    }
    fn failed(&self, q: &QueueRef) -> String {
        match self {
            Self::V5(k) => k.failed(q),
            Self::V6(k) => k.failed(q),
        }
    }
    fn delayed(&self, q: &QueueRef) -> String {
        match self {
            Self::V5(k) => k.delayed(q),
            Self::V6(k) => k.delayed(q),
        }
    }
    fn prioritized(&self, q: &QueueRef) -> String {
        match self {
            Self::V5(k) => k.prioritized(q),
            Self::V6(k) => k.prioritized(q),
        }
    }
    fn waiting_children(&self, q: &QueueRef) -> String {
        match self {
            Self::V5(k) => k.waiting_children(q),
            Self::V6(k) => k.waiting_children(q),
        }
    }
    fn stalled(&self, q: &QueueRef) -> String {
        match self {
            Self::V5(k) => k.stalled(q),
            Self::V6(k) => k.stalled(q),
        }
    }
    fn job_hash(&self, jref: &JobRef) -> String {
        match self {
            Self::V5(k) => k.job_hash(jref),
            Self::V6(k) => k.job_hash(jref),
        }
    }
    fn meta(&self, q: &QueueRef) -> String {
        match self {
            Self::V5(k) => k.meta(q),
            Self::V6(k) => k.meta(q),
        }
    }
    fn paused(&self, q: &QueueRef) -> String {
        match self {
            Self::V5(k) => k.paused(q),
            Self::V6(k) => k.paused(q),
        }
    }
    fn repeat(&self, q: &QueueRef) -> String {
        match self {
            Self::V5(k) => k.repeat(q),
            Self::V6(k) => k.repeat(q),
        }
    }
    fn logs(&self, jref: &JobRef) -> String {
        match self {
            Self::V5(k) => k.logs(jref),
            Self::V6(k) => k.logs(jref),
        }
    }
}
