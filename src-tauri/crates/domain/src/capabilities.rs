//! What a queue backend can do, as plain data.
//!
//! A kind of backend declares its capabilities once, on its spec, and
//! each backend of that kind answers the same value; operations read them
//! before they call it and answer `Unsupported` for what is not declared,
//! so a backend never receives a call it did not sign up for. Each family
//! of the port has its own small record, one flag per variant. A family
//! is provided (its accessor on `QueueBackend` is `Some`) exactly when at
//! least one of its flags is on; the backend contract suite checks it.
//!
//! [`Capability`] names one flag (`schedules.pause`): what an
//! `Unsupported` error names, what an MCP tool needs, and the path of the
//! flag in the descriptor the front end reads.

use crate::errors::RanchError;
use crate::jobs::JobState;
use crate::maintenance::QueueMaintenance;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// The job states the backend keeps: the common lifecycle it uses and
    /// its own states, each described. Listing any other state is refused.
    pub job_states: Vec<JobState>,
    pub job_actions: JobActionSupport,
    pub maintenance: MaintenanceSupport,
    /// Pausing and resuming a queue.
    pub queue_control: bool,
    pub schedules: ScheduleSupport,
    pub workers: WorkerSupport,
    /// Parent and child jobs.
    pub flows: bool,
    pub metrics: MetricsSupport,
    pub events: EventSupport,
    pub server: ServerSupport,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JobActionSupport {
    /// Retry, remove and promote one job.
    pub single: bool,
    /// The same on many jobs of a queue, as one action.
    pub bulk: bool,
    pub add: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MaintenanceSupport {
    pub drain: bool,
    pub clean: bool,
    pub retry_all_failed: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScheduleSupport {
    pub list: bool,
    /// Create, replace and remove schedulers.
    pub write: bool,
    /// Pause and resume a scheduler without removing it.
    pub pause: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkerSupport {
    pub list: bool,
    pub count: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MetricsSupport {
    /// Rates and durations of one queue.
    pub queue: bool,
    pub throughput: bool,
    /// Summaries of a window of the timeline.
    pub timeline: bool,
    /// The backend keeps metrics of its own. Without them, rates are
    /// estimated from the jobs still stored.
    pub native: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EventSupport {
    /// Reading the recent events of the queues.
    pub read: bool,
    /// Following them live, so windows refresh what moved instead of
    /// polling everything.
    pub follow: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServerSupport {
    pub info: bool,
    pub topology: bool,
}

impl JobActionSupport {
    pub fn any(&self) -> bool {
        self.single || self.bulk || self.add
    }
}

impl MaintenanceSupport {
    pub fn any(&self) -> bool {
        self.drain || self.clean || self.retry_all_failed
    }
}

impl ScheduleSupport {
    pub fn any(&self) -> bool {
        self.list || self.write || self.pause
    }
}

impl WorkerSupport {
    pub fn any(&self) -> bool {
        self.list || self.count
    }
}

impl MetricsSupport {
    /// `native` qualifies the others: alone it provides nothing.
    pub fn any(&self) -> bool {
        self.queue || self.throughput || self.timeline
    }
}

impl EventSupport {
    pub fn any(&self) -> bool {
        self.read || self.follow
    }
}

impl ServerSupport {
    pub fn any(&self) -> bool {
        self.info || self.topology
    }
}

/// One flag of [`Capabilities`], by the name errors, the MCP tool table
/// and the front end use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    JobActionsSingle,
    JobActionsBulk,
    JobActionsAdd,
    MaintenanceDrain,
    MaintenanceClean,
    MaintenanceRetryAllFailed,
    QueueControl,
    SchedulesList,
    SchedulesWrite,
    SchedulesPause,
    WorkersList,
    WorkersCount,
    Flows,
    MetricsQueue,
    MetricsThroughput,
    MetricsTimeline,
    EventsRead,
    EventsFollow,
    ServerInfo,
    ServerTopology,
}

impl Capability {
    pub const ALL: [Capability; 20] = [
        Capability::JobActionsSingle,
        Capability::JobActionsBulk,
        Capability::JobActionsAdd,
        Capability::MaintenanceDrain,
        Capability::MaintenanceClean,
        Capability::MaintenanceRetryAllFailed,
        Capability::QueueControl,
        Capability::SchedulesList,
        Capability::SchedulesWrite,
        Capability::SchedulesPause,
        Capability::WorkersList,
        Capability::WorkersCount,
        Capability::Flows,
        Capability::MetricsQueue,
        Capability::MetricsThroughput,
        Capability::MetricsTimeline,
        Capability::EventsRead,
        Capability::EventsFollow,
        Capability::ServerInfo,
        Capability::ServerTopology,
    ];

    /// The flag's path: its family and variant (`schedules.pause`), or
    /// the family alone when it has one flag (`flows`).
    pub fn name(self) -> &'static str {
        match self {
            Capability::JobActionsSingle => "jobActions.single",
            Capability::JobActionsBulk => "jobActions.bulk",
            Capability::JobActionsAdd => "jobActions.add",
            Capability::MaintenanceDrain => "maintenance.drain",
            Capability::MaintenanceClean => "maintenance.clean",
            Capability::MaintenanceRetryAllFailed => "maintenance.retryAllFailed",
            Capability::QueueControl => "queueControl",
            Capability::SchedulesList => "schedules.list",
            Capability::SchedulesWrite => "schedules.write",
            Capability::SchedulesPause => "schedules.pause",
            Capability::WorkersList => "workers.list",
            Capability::WorkersCount => "workers.count",
            Capability::Flows => "flows",
            Capability::MetricsQueue => "metrics.queue",
            Capability::MetricsThroughput => "metrics.throughput",
            Capability::MetricsTimeline => "metrics.timeline",
            Capability::EventsRead => "events.read",
            Capability::EventsFollow => "events.follow",
            Capability::ServerInfo => "server.info",
            Capability::ServerTopology => "server.topology",
        }
    }

    /// What a maintenance operation needs.
    pub fn of_maintenance(operation: &QueueMaintenance) -> Capability {
        match operation {
            QueueMaintenance::Drain { .. } => Capability::MaintenanceDrain,
            QueueMaintenance::Clean { .. } => Capability::MaintenanceClean,
            QueueMaintenance::RetryAllFailed => Capability::MaintenanceRetryAllFailed,
        }
    }
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl Capabilities {
    /// Whether the backend declares `capability`.
    pub fn supports(&self, capability: Capability) -> bool {
        match capability {
            Capability::JobActionsSingle => self.job_actions.single,
            Capability::JobActionsBulk => self.job_actions.bulk,
            Capability::JobActionsAdd => self.job_actions.add,
            Capability::MaintenanceDrain => self.maintenance.drain,
            Capability::MaintenanceClean => self.maintenance.clean,
            Capability::MaintenanceRetryAllFailed => self.maintenance.retry_all_failed,
            Capability::QueueControl => self.queue_control,
            Capability::SchedulesList => self.schedules.list,
            Capability::SchedulesWrite => self.schedules.write,
            Capability::SchedulesPause => self.schedules.pause,
            Capability::WorkersList => self.workers.list,
            Capability::WorkersCount => self.workers.count,
            Capability::Flows => self.flows,
            Capability::MetricsQueue => self.metrics.queue,
            Capability::MetricsThroughput => self.metrics.throughput,
            Capability::MetricsTimeline => self.metrics.timeline,
            Capability::EventsRead => self.events.read,
            Capability::EventsFollow => self.events.follow,
            Capability::ServerInfo => self.server.info,
            Capability::ServerTopology => self.server.topology,
        }
    }

    /// `Ok` when the backend declares `capability`, `Unsupported` naming
    /// it otherwise.
    pub fn ensure(&self, capability: Capability) -> Result<(), RanchError> {
        ensure(self.supports(capability), capability.name())
    }

    /// The state `code` names among those the backend keeps, or
    /// `Unsupported` naming the code.
    pub fn job_state(&self, code: &str) -> Result<JobState, RanchError> {
        JobState::from_code(code, &self.job_states).ok_or_else(|| RanchError::Unsupported {
            capability: format!("jobState.{code}"),
        })
    }
}

/// `Ok` when the backend declared the capability, `Unsupported` naming it
/// otherwise.
pub fn ensure(declared: bool, name: &str) -> Result<(), RanchError> {
    if declared {
        Ok(())
    } else {
        Err(RanchError::Unsupported {
            capability: name.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maintenance::CleanState;

    #[test]
    fn a_maintenance_operation_names_itself_when_refused() {
        let only_drain = Capabilities {
            maintenance: MaintenanceSupport {
                drain: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let drain = QueueMaintenance::Drain {
            include_delayed: true,
        };
        assert!(only_drain
            .ensure(Capability::of_maintenance(&drain))
            .is_ok());
        let clean = QueueMaintenance::Clean {
            state: CleanState::Completed,
            older_than_ms: 0,
        };
        let refused = only_drain.ensure(Capability::of_maintenance(&clean));
        assert!(matches!(
            refused,
            Err(RanchError::Unsupported { capability }) if capability == "maintenance.clean"
        ));
    }

    #[test]
    fn a_state_the_backend_does_not_keep_is_refused_by_its_code() {
        use crate::jobs::{BackendState, Tone};
        let held = JobState::Other(BackendState {
            code: "held",
            label: "Held",
            tone: Tone::Warning,
        });
        let caps = Capabilities {
            job_states: vec![JobState::Waiting, JobState::Failed, held],
            ..Default::default()
        };
        assert_eq!(caps.job_state("failed").unwrap(), JobState::Failed);
        assert_eq!(caps.job_state("held").unwrap(), held);
        assert!(matches!(
            caps.job_state("delayed"),
            Err(RanchError::Unsupported { capability }) if capability == "jobState.delayed"
        ));
    }

    #[test]
    fn each_capability_names_one_flag() {
        let names: std::collections::HashSet<_> =
            Capability::ALL.iter().map(|c| c.name()).collect();
        assert_eq!(names.len(), Capability::ALL.len());
        let none = Capabilities::default();
        assert!(Capability::ALL.iter().all(|c| !none.supports(*c)));
        let pause_only = Capabilities {
            schedules: ScheduleSupport {
                pause: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let supported: Vec<_> = Capability::ALL
            .into_iter()
            .filter(|c| pause_only.supports(*c))
            .collect();
        assert_eq!(supported, [Capability::SchedulesPause]);
    }
}
