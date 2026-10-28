//! A queue, its counts, and how it is named.

use std::collections::BTreeMap;

use crate::jobs::JobState;

/// A queue as operations name it. Opaque: the backend that listed the
/// queue made it and is the only one that reads it back, so nothing else
/// builds one or takes it apart.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QueueId(String);

impl QueueId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for QueueId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// How many jobs a queue holds in each state, by state code. A backend
/// counts the states it keeps; a state it did not count reads as zero.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StateCounts(BTreeMap<String, u64>);

impl StateCounts {
    pub fn set(&mut self, state: JobState, count: u64) {
        self.0.insert(state.as_str().to_string(), count);
    }

    /// The jobs in `state`, zero when it was not counted.
    pub fn get(&self, state: JobState) -> u64 {
        self.0.get(state.as_str()).copied().unwrap_or(0)
    }

    /// Every job counted, whatever its state.
    pub fn total(&self) -> u64 {
        self.0.values().sum()
    }

    /// The counts by state code, in code order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, u64)> {
        self.0.iter().map(|(code, n)| (code.as_str(), *n))
    }
}

impl FromIterator<(JobState, u64)> for StateCounts {
    fn from_iter<I: IntoIterator<Item = (JobState, u64)>>(iter: I) -> Self {
        let mut counts = Self::default();
        for (state, n) in iter {
            counts.set(state, n);
        }
        counts
    }
}

/// A queue of a connection, its counts and whether it is paused.
#[derive(Debug, Clone, PartialEq)]
pub struct Queue {
    pub id: QueueId,
    /// What the queue is called, for display.
    pub name: String,
    pub counts: StateCounts,
    pub is_paused: bool,
}

/// Where a queue stands after a pause or a resume. `already_in_state` is
/// true when it was there before the call, which then changed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueActionResult {
    pub is_paused: bool,
    pub already_in_state: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_not_counted_reads_as_zero_and_the_total_adds_every_state() {
        let counts: StateCounts = [(JobState::Waiting, 3), (JobState::Failed, 2)]
            .into_iter()
            .collect();
        assert_eq!(counts.get(JobState::Waiting), 3);
        assert_eq!(counts.get(JobState::Delayed), 0);
        assert_eq!(counts.total(), 5);
        assert_eq!(
            counts.iter().collect::<Vec<_>>(),
            vec![("failed", 2), ("waiting", 3)]
        );
    }
}
