//! What a kind of backend can do, as the front end reads it: one record
//! per family with a flag per variant, and the job states it keeps, each
//! described. It mirrors the domain's `Capabilities`; a flag's path here
//! (`schedules.pause`) is the name of the `Capability` an `unsupported`
//! error gives.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::jobs::JobState;
use crate::domain;
use crate::domain::capabilities::Capabilities;

/// What every connection of a kind can do, known without connecting. A
/// screen offers what is declared; an operation still refuses the rest.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDescriptor {
    /// The states the backend keeps, in the order a queue shows them.
    pub job_states: Vec<JobStateInfo>,
    pub job_actions: JobActionCapabilities,
    pub maintenance: MaintenanceCapabilities,
    /// Pausing and resuming a queue.
    pub queue_control: bool,
    pub schedules: ScheduleCapabilities,
    pub workers: WorkerCapabilities,
    /// Parent and child jobs.
    pub flows: bool,
    pub metrics: MetricsCapabilities,
    pub events: EventCapabilities,
    pub server: ServerCapabilities,
}

/// A job state as screens show it: its code, its label and its tone.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobStateInfo {
    pub code: JobState,
    pub label: String,
    pub tone: StateTone,
    /// One of the lifecycle every backend keeps, rather than the
    /// backend's own.
    pub common: bool,
}

/// The look a state borrows from the lifecycle's: pending as waiting,
/// running as active, success as completed, warning as delayed, danger
/// as failed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StateTone {
    Neutral,
    Pending,
    Running,
    Success,
    Warning,
    Danger,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobActionCapabilities {
    /// Retry, remove and promote one job.
    pub single: bool,
    /// The same on many jobs of a queue, as one action.
    pub bulk: bool,
    pub add: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceCapabilities {
    pub drain: bool,
    pub clean: bool,
    pub retry_all_failed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleCapabilities {
    pub list: bool,
    /// Create, replace and remove schedulers.
    pub write: bool,
    /// Pause and resume a scheduler without removing it.
    pub pause: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkerCapabilities {
    pub list: bool,
    pub count: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MetricsCapabilities {
    /// Rates and durations of one queue.
    pub queue: bool,
    pub throughput: bool,
    /// Summaries of a window of the timeline.
    pub timeline: bool,
    /// The backend keeps metrics of its own; without them, rates are
    /// estimated from the jobs still stored.
    pub native: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EventCapabilities {
    pub read: bool,
    /// Followed live, so windows refresh what moved instead of polling.
    pub follow: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    pub info: bool,
    pub topology: bool,
}

impl From<&Capabilities> for CapabilityDescriptor {
    fn from(c: &Capabilities) -> Self {
        Self {
            job_states: c.job_states.iter().copied().map(Into::into).collect(),
            job_actions: JobActionCapabilities {
                single: c.job_actions.single,
                bulk: c.job_actions.bulk,
                add: c.job_actions.add,
            },
            maintenance: MaintenanceCapabilities {
                drain: c.maintenance.drain,
                clean: c.maintenance.clean,
                retry_all_failed: c.maintenance.retry_all_failed,
            },
            queue_control: c.queue_control,
            schedules: ScheduleCapabilities {
                list: c.schedules.list,
                write: c.schedules.write,
                pause: c.schedules.pause,
            },
            workers: WorkerCapabilities {
                list: c.workers.list,
                count: c.workers.count,
            },
            flows: c.flows,
            metrics: MetricsCapabilities {
                queue: c.metrics.queue,
                throughput: c.metrics.throughput,
                timeline: c.metrics.timeline,
                native: c.metrics.native,
            },
            events: EventCapabilities {
                read: c.events.read,
                follow: c.events.follow,
            },
            server: ServerCapabilities {
                info: c.server.info,
                topology: c.server.topology,
            },
        }
    }
}

impl From<domain::jobs::JobState> for JobStateInfo {
    fn from(state: domain::jobs::JobState) -> Self {
        Self {
            code: state.into(),
            label: state.label().to_string(),
            tone: state.tone().into(),
            common: state.is_common(),
        }
    }
}

impl From<domain::jobs::Tone> for StateTone {
    fn from(tone: domain::jobs::Tone) -> Self {
        match tone {
            domain::jobs::Tone::Neutral => Self::Neutral,
            domain::jobs::Tone::Pending => Self::Pending,
            domain::jobs::Tone::Running => Self::Running,
            domain::jobs::Tone::Success => Self::Success,
            domain::jobs::Tone::Warning => Self::Warning,
            domain::jobs::Tone::Danger => Self::Danger,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::capabilities::Capability;

    /// Every flag of the descriptor sits at the path its `Capability`
    /// names, so the front end and the errors speak of the same thing.
    #[test]
    fn each_capability_is_the_flag_at_its_path() {
        for capability in Capability::ALL {
            let mut caps = Capabilities::default();
            let json = serde_json::to_value(CapabilityDescriptor::from(&caps)).unwrap();
            let at = |json: &serde_json::Value| {
                capability
                    .name()
                    .split('.')
                    .fold(json.clone(), |v, key| v[key].clone())
            };
            assert_eq!(at(&json), false, "{capability} is off by default");
            set(&mut caps, capability);
            assert!(caps.supports(capability));
            let json = serde_json::to_value(CapabilityDescriptor::from(&caps)).unwrap();
            assert_eq!(at(&json), true, "{capability} is at its path");
        }
    }

    fn set(c: &mut Capabilities, capability: Capability) {
        match capability {
            Capability::JobActionsSingle => c.job_actions.single = true,
            Capability::JobActionsBulk => c.job_actions.bulk = true,
            Capability::JobActionsAdd => c.job_actions.add = true,
            Capability::MaintenanceDrain => c.maintenance.drain = true,
            Capability::MaintenanceClean => c.maintenance.clean = true,
            Capability::MaintenanceRetryAllFailed => c.maintenance.retry_all_failed = true,
            Capability::QueueControl => c.queue_control = true,
            Capability::SchedulesList => c.schedules.list = true,
            Capability::SchedulesWrite => c.schedules.write = true,
            Capability::SchedulesPause => c.schedules.pause = true,
            Capability::WorkersList => c.workers.list = true,
            Capability::WorkersCount => c.workers.count = true,
            Capability::Flows => c.flows = true,
            Capability::MetricsQueue => c.metrics.queue = true,
            Capability::MetricsThroughput => c.metrics.throughput = true,
            Capability::MetricsTimeline => c.metrics.timeline = true,
            Capability::EventsRead => c.events.read = true,
            Capability::EventsFollow => c.events.follow = true,
            Capability::ServerInfo => c.server.info = true,
            Capability::ServerTopology => c.server.topology = true,
        }
    }
}
