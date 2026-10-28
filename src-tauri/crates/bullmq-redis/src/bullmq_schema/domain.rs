//! The domain types the BullMQ schema decodes into, and which Redis
//! structure holds the jobs of each state.

// The decoders produce the domain's queues, jobs and schedulers.
use ranch_domain::errors::RanchError;
use ranch_domain::jobs::{BackendState, Tone};
pub use ranch_domain::jobs::{Job, JobRef, JobState, RecentJobs};
pub use ranch_domain::queues::QueueId;
pub use ranch_domain::schedule::CronJob;

/// A BullMQ queue: its key prefix (`bull` unless the app set another) and
/// its name. Its `QueueId` is `{prefix}:{name}`, the base of every key of
/// the queue, and is read back by splitting on the last colon, as
/// discovery splits a `…:meta` key: unambiguous for any queue Ranch lists.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QueueRef {
    pub prefix: String,
    pub name: String,
}

impl QueueRef {
    pub fn id(&self) -> QueueId {
        QueueId::new(format!("{}:{}", self.prefix, self.name))
    }

    /// The queue an id made by [`QueueRef::id`] names.
    pub fn from_id(id: &QueueId) -> Result<Self, RanchError> {
        Self::parse(id.as_str())
    }

    /// [`QueueRef::from_id`] on the id's text, as an argument carries it.
    pub fn parse(id: &str) -> Result<Self, RanchError> {
        id.rsplit_once(':')
            .filter(|(prefix, name)| !prefix.is_empty() && !name.is_empty())
            .map(|(prefix, name)| Self {
                prefix: prefix.to_string(),
                name: name.to_string(),
            })
            .ok_or_else(|| RanchError::InvalidInput {
                reason: format!("'{id}' is not a queue id: take one from the queue list"),
            })
    }

    /// One of the queue's jobs.
    pub fn job(&self, id: impl Into<String>) -> JobRef {
        JobRef {
            queue_id: self.id(),
            queue_name: self.name.clone(),
            id: id.into(),
        }
    }
}

/// The job states BullMQ adds to the common lifecycle. Their codes are
/// BullMQ's own camelCase names.
pub mod states {
    use super::{BackendState, JobState, RanchError, Tone};

    /// Jobs added with a `priority`: their own sorted set, processed once
    /// the wait list is empty.
    pub const PRIORITIZED: JobState = JobState::Other(BackendState {
        code: "prioritized",
        label: "Prioritized",
        tone: Tone::Pending,
    });
    /// Flow parents whose children have not all completed.
    pub const WAITING_CHILDREN: JobState = JobState::Other(BackendState {
        code: "waitingChildren",
        label: "Waiting children",
        tone: Tone::Warning,
    });
    /// Jobs a worker stopped renewing the lock of, before BullMQ moves
    /// them back to wait or to failed.
    pub const STALLED: JobState = JobState::Other(BackendState {
        code: "stalled",
        label: "Stalled",
        tone: Tone::Danger,
    });
    /// What a decoder answers when a job hash matches no known encoding.
    /// Never listed.
    pub const UNKNOWN: JobState = JobState::Other(BackendState {
        code: "unknown",
        label: "Unknown",
        tone: Tone::Neutral,
    });

    /// Every state a BullMQ queue keeps jobs in: the common lifecycle and
    /// BullMQ's own, in the order a queue's tabs show them.
    pub const LISTED: [JobState; 9] = [
        JobState::Waiting,
        JobState::Active,
        JobState::Completed,
        JobState::Failed,
        JobState::Delayed,
        PRIORITIZED,
        WAITING_CHILDREN,
        STALLED,
        JobState::Paused,
    ];

    /// The state `code` names, `unknown` included.
    pub fn from_code(code: &str) -> Option<JobState> {
        JobState::from_code(code, &LISTED).or((code == UNKNOWN.as_str()).then_some(UNKNOWN))
    }

    /// The state `code` names, or `Unsupported` naming the code.
    pub fn require(code: &str) -> Result<JobState, RanchError> {
        from_code(code).ok_or_else(|| RanchError::Unsupported {
            capability: format!("jobState.{code}"),
        })
    }
}

/// Classifies the Redis structural type backing each
/// [`JobState`]. Drives whether `list_jobs` uses `LRANGE` (lists) or
/// `ZRANGE`/`ZREVRANGE` (sorted sets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateRedisShape {
    /// LRANGE iteration. BullMQ v6+ lists carry a `MARKER_TOKEN` sentinel
    /// at the head (see [`crate::bullmq_schema::v6::MARKER_TOKEN`])
    /// that callers MUST filter out before HGETALL.
    List,
    /// ZRANGE / ZREVRANGE iteration. Score is the relevant timestamp
    /// (e.g. `failedOn` for `Failed`, scheduled timestamp for `Delayed`).
    SortedSet,
    /// Schema-not-recognized fallback or a state we don't know how to
    /// fetch: `list_jobs` returns an empty page.
    Unsupported,
}

impl StateRedisShape {
    /// The shape of the structure that holds `state`'s jobs.
    pub fn of(state: JobState) -> Self {
        match state {
            JobState::Waiting | JobState::Active | JobState::Paused => Self::List,
            JobState::Completed
            | JobState::Failed
            | JobState::Delayed
            | states::PRIORITIZED
            | states::WAITING_CHILDREN
            | states::STALLED => Self::SortedSet,
            JobState::Other(_) => Self::Unsupported,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_states_are_classified_correctly() {
        assert_eq!(
            StateRedisShape::of(JobState::Waiting),
            StateRedisShape::List
        );
        assert_eq!(StateRedisShape::of(JobState::Active), StateRedisShape::List);
        assert_eq!(StateRedisShape::of(JobState::Paused), StateRedisShape::List);
    }

    #[test]
    fn sorted_set_states_are_classified_correctly() {
        for s in [
            JobState::Completed,
            JobState::Failed,
            JobState::Delayed,
            states::PRIORITIZED,
            states::WAITING_CHILDREN,
            states::STALLED,
        ] {
            assert_eq!(
                StateRedisShape::of(s),
                StateRedisShape::SortedSet,
                "state: {s:?}"
            );
        }
    }

    #[test]
    fn unknown_state_is_unsupported() {
        assert_eq!(
            StateRedisShape::of(states::UNKNOWN),
            StateRedisShape::Unsupported
        );
    }

    #[test]
    fn a_queue_id_is_the_key_base_and_reads_back_on_its_last_colon() {
        let q = QueueRef {
            prefix: "myapp:queues".into(),
            name: "billing".into(),
        };
        assert_eq!(q.id().as_str(), "myapp:queues:billing");
        assert_eq!(QueueRef::from_id(&q.id()).unwrap(), q);
        for bad in ["emails", ":emails", "bull:", ""] {
            assert!(
                matches!(
                    QueueRef::from_id(&QueueId::new(bad)),
                    Err(RanchError::InvalidInput { .. })
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn every_bullmq_state_is_found_by_its_code() {
        for s in states::LISTED.into_iter().chain([states::UNKNOWN]) {
            assert_eq!(states::from_code(s.as_str()), Some(s));
        }
        assert_eq!(states::from_code("waiting-children"), None);
    }
}
