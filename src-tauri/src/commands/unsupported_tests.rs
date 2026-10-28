//! Operations refuse what a backend does not declare, before calling it.
//! The stub below panics on every call: a refusal that reached the
//! backend would fail the test.

use std::sync::Arc;

use async_trait::async_trait;

use crate::adapters::bullmq_redis::bullmq_schema::{
    states, CronJob, Job, JobState, RecentJobs, SchedulerSaved,
};
use crate::adapters::progress::NoProgress;
use crate::application::ports::backend::{ConnectionControl, JobReads, QueueReads, Schedules};
use crate::application::ports::{BackendSpecs, Backends, QueueBackend};
use crate::application::ConnectionStamp;
use crate::commands::jobs::{list_jobs_inner, retry_job_inner, JobActionArgs, ListJobsArgs};
use crate::commands::queues::{
    follow_connection_events_inner, pause_queue_inner, pause_scheduler_inner,
    FollowConnectionEventsArgs, QueueActionArgs, SchedulerActionArgs,
};
use crate::domain::audit::Caller;
use crate::domain::capabilities::{Capabilities, ScheduleSupport};
use crate::domain::errors::RanchError;
use crate::domain::failures::{FailedJobsPage, FailedJobsQuery, FailureCauses};
use crate::domain::jobs::{
    JobLogs, JobLogsQuery, JobPage, JobPageQuery, JobTarget, RecentJobsQuery,
};
use crate::domain::lock::ReadOnlyState;
use crate::domain::probe::PingReport;
use crate::domain::queues::{Queue, QueueId};
use crate::domain::schedule::{SchedulerActionResult, SchedulerSpec, SchedulerTarget};

/// Lists schedulers, keeps waiting and failed jobs, and nothing else. It
/// provides the schedules family, so only its declaration refuses a
/// scheduler pause.
struct ListsOnly {
    stamp: ConnectionStamp,
    capabilities: Capabilities,
}

impl ListsOnly {
    fn backends() -> impl Backends {
        struct Open(BackendSpecs);
        #[async_trait]
        impl Backends for Open {
            fn specs(&self) -> &BackendSpecs {
                &self.0
            }

            async fn open(&self, id: &str) -> Result<Arc<dyn QueueBackend>, RanchError> {
                Ok(Arc::new(ListsOnly {
                    stamp: ConnectionStamp {
                        id: id.into(),
                        name: "stub".into(),
                        env_color: None,
                    },
                    capabilities: Capabilities {
                        job_states: vec![JobState::Waiting, JobState::Failed],
                        schedules: ScheduleSupport {
                            list: true,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                }))
            }
        }
        Open(BackendSpecs::default())
    }
}

const CALLED: &str = "an undeclared capability reached the backend";

impl QueueBackend for ListsOnly {
    fn stamp(&self) -> &ConnectionStamp {
        &self.stamp
    }
    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }
    fn connection(&self) -> &dyn ConnectionControl {
        self
    }
    fn queues(&self) -> &dyn QueueReads {
        self
    }
    fn jobs(&self) -> &dyn JobReads {
        self
    }
    fn schedules(&self) -> Option<&dyn Schedules> {
        Some(self)
    }
}

#[async_trait]
impl ConnectionControl for ListsOnly {
    async fn ping(&self) -> Result<PingReport, RanchError> {
        unreachable!("{CALLED}")
    }
    async fn lock_state(&self) -> Result<ReadOnlyState, RanchError> {
        unreachable!("{CALLED}")
    }
    async fn lock(&self) -> Result<ReadOnlyState, RanchError> {
        unreachable!("{CALLED}")
    }
    async fn unlock(&self) -> Result<ReadOnlyState, RanchError> {
        unreachable!("{CALLED}")
    }
}

#[async_trait]
impl QueueReads for ListsOnly {
    async fn list_queues(&self) -> Result<Vec<Queue>, RanchError> {
        unreachable!("{CALLED}")
    }
}

#[async_trait]
impl JobReads for ListsOnly {
    async fn list_jobs(&self, _: JobPageQuery) -> Result<JobPage, RanchError> {
        unreachable!("{CALLED}")
    }
    async fn get_job(&self, _: JobTarget) -> Result<Job, RanchError> {
        unreachable!("{CALLED}")
    }
    async fn get_job_logs(&self, _: JobLogsQuery) -> Result<JobLogs, RanchError> {
        unreachable!("{CALLED}")
    }
    async fn list_recent_jobs(&self, _: RecentJobsQuery) -> Result<RecentJobs, RanchError> {
        unreachable!("{CALLED}")
    }
    async fn list_failed_jobs(&self, _: FailedJobsQuery) -> Result<FailedJobsPage, RanchError> {
        unreachable!("{CALLED}")
    }
    async fn list_failure_causes(&self, _: Option<String>) -> Result<FailureCauses, RanchError> {
        unreachable!("{CALLED}")
    }
}

#[async_trait]
impl Schedules for ListsOnly {
    async fn list_schedulers(&self, _: QueueId) -> Result<Vec<CronJob>, RanchError> {
        unreachable!("{CALLED}")
    }
    async fn upsert_scheduler(
        &self,
        _: QueueId,
        _: SchedulerSpec,
        _: Caller,
    ) -> Result<SchedulerSaved, RanchError> {
        unreachable!("{CALLED}")
    }
    async fn remove_scheduler(
        &self,
        _: SchedulerTarget,
        _: Caller,
    ) -> Result<SchedulerActionResult, RanchError> {
        unreachable!("{CALLED}")
    }
    async fn pause_scheduler(
        &self,
        _: SchedulerTarget,
        _: Caller,
    ) -> Result<SchedulerActionResult, RanchError> {
        unreachable!("{CALLED}")
    }
    async fn resume_scheduler(
        &self,
        _: SchedulerTarget,
        _: Caller,
    ) -> Result<SchedulerActionResult, RanchError> {
        unreachable!("{CALLED}")
    }
}

fn queue() -> crate::contract::queues::QueueId {
    crate::contract::queues::QueueId("bull:emails".into())
}

fn refused_capability<T: std::fmt::Debug>(result: Result<T, RanchError>) -> String {
    match result {
        Err(RanchError::Unsupported { capability }) => capability,
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[tokio::test]
async fn a_declared_family_still_refuses_an_undeclared_variant() {
    let args = SchedulerActionArgs {
        connection_id: "c".into(),
        queue_id: queue(),
        scheduler_key: "nightly".into(),
    };
    let refused = pause_scheduler_inner(args, &ListsOnly::backends(), Caller::User).await;
    assert_eq!(refused_capability(refused), "schedules.pause");
}

#[tokio::test]
async fn a_missing_family_is_refused_by_name() {
    let backends = ListsOnly::backends();
    let pause = QueueActionArgs {
        connection_id: "c".into(),
        queue_id: queue(),
    };
    let refused = pause_queue_inner(pause, &backends, Caller::User).await;
    assert_eq!(refused_capability(refused), "queueControl");

    let retry = JobActionArgs {
        connection_id: "c".into(),
        queue_id: queue(),
        job_id: "1".into(),
    };
    let refused = retry_job_inner(retry, &backends, Caller::User).await;
    assert_eq!(refused_capability(refused), "jobActions.single");
}

#[tokio::test]
async fn a_job_state_the_backend_does_not_keep_is_refused() {
    let args = ListJobsArgs {
        connection_id: "c".into(),
        queue_id: queue(),
        state: states::PRIORITIZED.into(),
        offset: 0,
        limit: 50,
        newest_first: true,
        search: None,
        reason: None,
        sort: None,
    };
    let refused = list_jobs_inner(args, &ListsOnly::backends()).await;
    assert_eq!(refused_capability(refused), "jobState.prioritized");
}

#[tokio::test]
async fn a_backend_without_live_events_is_simply_not_followed() {
    let args = FollowConnectionEventsArgs {
        connection_id: "c".into(),
        follow: true,
    };
    let followed =
        follow_connection_events_inner(args, &ListsOnly::backends(), NoProgress::sink()).await;
    assert!(!followed.unwrap());
}
