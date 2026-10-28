//! One suite for every queue backend: what the port promises, checked the
//! same way on each. `tests/backend_contract.rs` runs it on the in-memory
//! backend, `tests/backend_contract_live.rs` on BullMQ over Redis.
//!
//! It asserts only what every backend must agree on: results and errors
//! of the operations, counters, the lock, and the audit rows writes leave.
//! A backend gives the suite a [`Seeder`] to create connections and queues
//! in its own store and to read its audit log back.

use async_trait::async_trait;
use ranch_lib::adapters::bullmq_redis::bullmq_schema::JobState;
use ranch_lib::application::ports::backend::QueueBackend;
use ranch_lib::application::ports::Backends;
use ranch_lib::domain::audit::{AuditAction, AuditOutcome, AuditTarget, Caller};
use ranch_lib::domain::errors::RanchError;
use ranch_lib::domain::failures::{FailedFilter, FailedJobsQuery};
use ranch_lib::domain::jobs::{JobBatch, JobPageQuery, JobTarget, NewJob, RecentJobsQuery};
use ranch_lib::domain::lock::ReadOnlyState;
use ranch_lib::domain::maintenance::{CleanState, QueueMaintenance};
use ranch_lib::domain::queues::{Queue, QueueId};
use ranch_lib::domain::schedule::{Schedule, SchedulerSpec, SchedulerTarget};

use JobState::{Active, Completed, Delayed, Failed, Waiting};

/// An audit row as the suite compares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRow {
    pub action: AuditAction,
    pub target: AuditTarget,
    pub outcome: AuditOutcome,
}

/// Sets up a backend's store for the suite.
// `async_trait` marks the boxed future `must_use` on top of its own.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait Seeder: Send + Sync {
    /// A new saved connection, locked or not; its id.
    async fn connection(&self, name: &str, locked: bool) -> String;

    /// Make the queue named `queue` exist and hold exactly `jobs`, each an
    /// id and one of waiting, active, completed, failed or delayed. Every
    /// job is named `test-job` and was queued (and finished, if it did)
    /// long ago; a failed one has `kaboom` as its reason. The queue's id,
    /// as the backend lists it.
    async fn queue(&self, connection_id: &str, queue: &str, jobs: &[(&str, JobState)]) -> QueueId;

    /// The id the backend would give a queue named `queue`, which does not
    /// have to exist.
    fn queue_id(&self, queue: &str) -> QueueId;

    /// The audit rows of `connection_id`, oldest first.
    async fn audit(&self, connection_id: &str) -> Vec<AuditRow>;
}

/// Every section, each on its own connection and queue.
pub async fn run_suite(backends: &dyn Backends, seeder: &dyn Seeder) {
    capabilities_describe_the_families(backends, seeder).await;
    connections(backends, seeder).await;
    reads(backends, seeder).await;
    single_job_actions(backends, seeder).await;
    adding_jobs(backends, seeder).await;
    bulk_actions(backends, seeder).await;
    pausing_a_queue(backends, seeder).await;
    maintenance(backends, seeder).await;
    schedules(backends, seeder).await;
    workers(backends, seeder).await;
    the_lock(backends, seeder).await;
}

fn job(queue: &QueueId, id: &str) -> JobTarget {
    JobTarget {
        queue_id: queue.clone(),
        job_id: id.into(),
    }
}

fn bulk(queue: &QueueId, ids: &[&str]) -> JobBatch {
    JobBatch {
        queue_id: queue.clone(),
        job_ids: ids.iter().map(|id| id.to_string()).collect(),
    }
}

fn list(queue: &QueueId, state: JobState) -> JobPageQuery {
    JobPageQuery {
        queue_id: queue.clone(),
        state,
        offset: 0,
        limit: 50,
        newest_first: true,
        search: None,
        reason: None,
        sort: None,
    }
}

fn row(action: AuditAction, target: AuditTarget, outcome: AuditOutcome) -> AuditRow {
    AuditRow {
        action,
        target,
        outcome,
    }
}

fn on_job(queue: &QueueId, name: &str, id: &str) -> AuditTarget {
    AuditTarget::job(queue, name, id)
}

fn on_queue(queue: &QueueId, name: &str) -> AuditTarget {
    AuditTarget::queue(queue, name)
}

fn on_scheduler(queue: &QueueId, name: &str, key: &str) -> AuditTarget {
    AuditTarget::scheduler(queue, name, key)
}

fn on_bulk(queue: &QueueId, name: &str, job_count: u32) -> AuditTarget {
    AuditTarget::bulk_jobs(queue, name, job_count)
}

async fn queue_named(backend: &dyn QueueBackend, id: &QueueId) -> Queue {
    let queues = backend.queues().list_queues().await.expect("list_queues");
    queues
        .into_iter()
        .find(|q| &q.id == id)
        .unwrap_or_else(|| panic!("queue {id} is listed"))
}

async fn state_of(backend: &dyn QueueBackend, queue: &QueueId, id: &str) -> JobState {
    backend
        .jobs()
        .get_job(job(queue, id))
        .await
        .unwrap_or_else(|e| panic!("job {id}: {e}"))
        .state
}

async fn ids_in(backend: &dyn QueueBackend, queue: &QueueId, state: JobState) -> Vec<String> {
    let mut ids: Vec<String> = backend
        .jobs()
        .list_jobs(list(queue, state))
        .await
        .expect("list_jobs")
        .jobs
        .into_iter()
        .map(|j| j.job_ref.id)
        .collect();
    ids.sort();
    ids
}

/// A queue's waiting, active, completed, failed and delayed jobs, checked
/// to be all it holds.
fn counts(queue: &Queue) -> [u64; 5] {
    let five = [
        JobState::Waiting,
        JobState::Active,
        JobState::Completed,
        JobState::Failed,
        JobState::Delayed,
    ]
    .map(|s| queue.counts.get(s));
    assert_eq!(
        queue.counts.total(),
        five.iter().sum::<u64>(),
        "jobs in another state: {:?}",
        queue.counts
    );
    five
}

/// A family is provided exactly when its capabilities declare a variant,
/// and the capabilities are those its kind declares without connecting.
pub async fn capabilities_describe_the_families(backends: &dyn Backends, seeder: &dyn Seeder) {
    let conn = seeder.connection("contract capabilities", false).await;
    let backend = backends.open(&conn).await.expect("open");
    let caps = backend.capabilities();
    assert!(
        backends
            .specs()
            .iter()
            .any(|spec| spec.capabilities() == caps),
        "a backend declares what its kind's spec declares"
    );
    let agree = [
        (
            "jobActions",
            caps.job_actions.any(),
            backend.job_actions().is_some(),
        ),
        (
            "maintenance",
            caps.maintenance.any(),
            backend.maintenance().is_some(),
        ),
        (
            "queueControl",
            caps.queue_control,
            backend.queue_control().is_some(),
        ),
        (
            "schedules",
            caps.schedules.any(),
            backend.schedules().is_some(),
        ),
        ("workers", caps.workers.any(), backend.workers().is_some()),
        ("flows", caps.flows, backend.flows().is_some()),
        ("metrics", caps.metrics.any(), backend.metrics().is_some()),
        ("events", caps.events.any(), backend.events().is_some()),
        ("server", caps.server.any(), backend.server().is_some()),
    ];
    for (family, declared, provided) in agree {
        assert_eq!(
            declared, provided,
            "{family}: declared {declared}, provided {provided}"
        );
    }
    for state in [Waiting, Active, Completed, Failed, Delayed] {
        assert!(caps.job_states.contains(&state), "{state:?} is kept");
    }
}

pub async fn connections(backends: &dyn Backends, seeder: &dyn Seeder) {
    let unknown = backends.open("no-such-connection").await;
    assert!(
        matches!(unknown, Err(RanchError::ConnectionNotFound { .. })),
        "an unknown id is ConnectionNotFound"
    );
    let conn = seeder.connection("contract connection", false).await;
    let backend = backends.open(&conn).await.expect("open");
    assert_eq!(backend.stamp().id, conn);
    let ping = backend.connection().ping().await.expect("ping");
    assert!(ping.success, "a reachable backend pings: {:?}", ping.error);
    assert_eq!(
        backend.connection().lock_state().await.unwrap(),
        ReadOnlyState::Unlocked
    );
}

pub async fn reads(backends: &dyn Backends, seeder: &dyn Seeder) {
    let q = "ranch-contract-reads";
    let conn = seeder.connection("contract reads", false).await;
    let jobs = [
        ("w1", Waiting),
        ("a1", Active),
        ("c1", Completed),
        ("f1", Failed),
        ("d1", Delayed),
    ];
    let qid = seeder.queue(&conn, q, &jobs).await;
    let backend = backends.open(&conn).await.expect("open");

    let queue = queue_named(backend.as_ref(), &qid).await;
    assert_eq!(counts(&queue), [1, 1, 1, 1, 1]);
    assert!(!queue.is_paused);

    for (id, state) in jobs {
        assert_eq!(ids_in(backend.as_ref(), &qid, state).await, [id]);
    }
    let failed = backend.jobs().get_job(job(&qid, "f1")).await.unwrap();
    assert_eq!(failed.name, "test-job");
    assert_eq!(failed.state, Failed);
    assert_eq!(failed.failed_reason.as_deref(), Some("kaboom"));
    let missing = backend.jobs().get_job(job(&qid, "nope")).await;
    assert!(
        matches!(missing, Err(RanchError::JobNotFound { .. })),
        "a missing job is JobNotFound: {missing:?}"
    );

    let recent = backend
        .jobs()
        .list_recent_jobs(RecentJobsQuery {
            queue_id: Some(qid.clone()),
            limit: 10,
        })
        .await
        .unwrap();
    let mut ran: Vec<&str> = recent.jobs.iter().map(|j| j.job_ref.id.as_str()).collect();
    assert_eq!(ran.first(), Some(&"a1"), "running jobs come first");
    ran.sort();
    assert_eq!(ran, ["a1", "c1", "f1"]);

    let page = backend
        .jobs()
        .list_failed_jobs(FailedJobsQuery {
            offset: 0,
            limit: 50,
            filter: FailedFilter {
                queue: Some(q.into()),
                ..FailedFilter::default()
            },
        })
        .await
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.jobs[0].job_ref.id, "f1");

    let causes = backend
        .jobs()
        .list_failure_causes(Some(q.into()))
        .await
        .unwrap();
    assert_eq!(causes.causes.len(), 1);
    assert_eq!(causes.causes[0].sample, "kaboom");
    assert_eq!(causes.causes[0].count, 1);
    assert!(seeder.audit(&conn).await.is_empty(), "reads leave no row");
}

pub async fn single_job_actions(backends: &dyn Backends, seeder: &dyn Seeder) {
    let q = "ranch-contract-actions";
    let conn = seeder.connection("contract actions", false).await;
    let qid = seeder
        .queue(
            &conn,
            q,
            &[("f1", Failed), ("d1", Delayed), ("w1", Waiting)],
        )
        .await;
    let backend = backends.open(&conn).await.expect("open");
    let actions = backend.job_actions().expect("job actions");

    let retried = actions
        .retry_job(job(&qid, "f1"), Caller::User)
        .await
        .unwrap();
    assert_eq!(
        (retried.action.as_str(), retried.new_state),
        ("retry", Some(Waiting))
    );
    assert_eq!(state_of(backend.as_ref(), &qid, "f1").await, Waiting);
    let wrong = actions.retry_job(job(&qid, "w1"), Caller::User).await;
    assert!(
        matches!(
            wrong,
            Err(RanchError::InvalidJobState {
                required_state: Failed,
                ..
            })
        ),
        "only a failed job is retried: {wrong:?}"
    );

    let promoted = actions
        .promote_job(job(&qid, "d1"), Caller::User)
        .await
        .unwrap();
    assert_eq!(
        (promoted.action.as_str(), promoted.new_state),
        ("promote", Some(Waiting))
    );
    assert_eq!(state_of(backend.as_ref(), &qid, "d1").await, Waiting);
    let wrong = actions.promote_job(job(&qid, "w1"), Caller::User).await;
    assert!(
        matches!(
            wrong,
            Err(RanchError::InvalidJobState {
                required_state: Delayed,
                ..
            })
        ),
        "only a delayed job is promoted: {wrong:?}"
    );

    let removed = actions
        .remove_job(job(&qid, "w1"), Caller::User)
        .await
        .unwrap();
    assert_eq!(
        (removed.action.as_str(), removed.new_state),
        ("remove", None)
    );
    let gone = backend.jobs().get_job(job(&qid, "w1")).await;
    assert!(matches!(gone, Err(RanchError::JobNotFound { .. })));
    let again = actions.remove_job(job(&qid, "w1"), Caller::User).await;
    assert!(matches!(again, Err(RanchError::JobNotFound { .. })));

    assert_eq!(
        counts(&queue_named(backend.as_ref(), &qid).await),
        [2, 0, 0, 0, 0]
    );
    // Refusals before acting leave no row.
    assert_eq!(
        seeder.audit(&conn).await,
        [
            row(
                AuditAction::RetryJob,
                on_job(&qid, q, "f1"),
                AuditOutcome::Success
            ),
            row(
                AuditAction::PromoteJob,
                on_job(&qid, q, "d1"),
                AuditOutcome::Success
            ),
            row(
                AuditAction::RemoveJob,
                on_job(&qid, q, "w1"),
                AuditOutcome::Success
            ),
        ]
    );
}

fn new_job(name: &str, delay_ms: u64) -> NewJob {
    NewJob {
        name: name.into(),
        data: r#"{"to":"ada@example.com"}"#.into(),
        delay_ms,
        attempts: None,
        priority: 0,
        lifo: false,
        backoff: None,
        job_id: None,
        remove_on_complete: false,
        remove_on_fail: false,
    }
}

pub async fn adding_jobs(backends: &dyn Backends, seeder: &dyn Seeder) {
    let q = "ranch-contract-add";
    let conn = seeder.connection("contract add", false).await;
    let qid = seeder.queue(&conn, q, &[]).await;
    let backend = backends.open(&conn).await.expect("open");
    let actions = backend.job_actions().expect("job actions");

    let now = actions
        .add_job(qid.clone(), new_job("welcome", 0), Caller::User)
        .await
        .unwrap();
    assert_eq!(now.state, Waiting);
    let later = actions
        .add_job(qid.clone(), new_job("reminder", 60_000), Caller::User)
        .await
        .unwrap();
    assert_eq!(later.state, Delayed);
    assert_ne!(now.job_id, later.job_id);
    let added = backend
        .jobs()
        .get_job(job(&qid, &now.job_id))
        .await
        .unwrap();
    assert_eq!((added.name.as_str(), added.state), ("welcome", Waiting));

    let nowhere = actions
        .add_job(
            seeder.queue_id("ranch-contract-no-such-queue"),
            new_job("x", 0),
            Caller::User,
        )
        .await;
    assert!(
        matches!(nowhere, Err(RanchError::InvalidInput { .. })),
        "a job is never added to a queue that does not exist: {nowhere:?}"
    );
    let invalid = actions
        .add_job(qid.clone(), new_job("", 0), Caller::User)
        .await;
    assert!(matches!(invalid, Err(RanchError::InvalidInput { .. })));

    assert_eq!(
        seeder.audit(&conn).await,
        [
            row(
                AuditAction::AddJob,
                on_job(&qid, q, &now.job_id),
                AuditOutcome::Success
            ),
            row(
                AuditAction::AddJob,
                on_job(&qid, q, &later.job_id),
                AuditOutcome::Success
            ),
        ]
    );
}

pub async fn bulk_actions(backends: &dyn Backends, seeder: &dyn Seeder) {
    let q = "ranch-contract-bulk";
    let conn = seeder.connection("contract bulk", false).await;
    let qid = seeder
        .queue(&conn, q, &[("f1", Failed), ("f2", Failed), ("w1", Waiting)])
        .await;
    let backend = backends.open(&conn).await.expect("open");
    let actions = backend.job_actions().expect("job actions");

    let retried = actions
        .bulk_retry_jobs(
            bulk(&qid, &["f1", "f2", "w1"]),
            super::no_progress(),
            Caller::User,
        )
        .await
        .unwrap();
    assert_eq!(retried.action, "retry");
    assert_eq!((retried.total, retried.succeeded), (3, 2));
    assert_eq!(retried.failed.len(), 1);
    assert_eq!(retried.failed[0].job_id, "w1");
    assert!(!retried.cancelled);
    assert_eq!(
        counts(&queue_named(backend.as_ref(), &qid).await),
        [3, 0, 0, 0, 0]
    );

    let removed = actions
        .bulk_remove_jobs(
            bulk(&qid, &["f1", "f2"]),
            super::no_progress(),
            Caller::User,
        )
        .await
        .unwrap();
    assert_eq!((removed.action.as_str(), removed.succeeded), ("remove", 2));
    assert_eq!(ids_in(backend.as_ref(), &qid, Waiting).await, ["w1"]);

    // One row per bulk action, a failure when any job failed.
    assert_eq!(
        seeder.audit(&conn).await,
        [
            row(
                AuditAction::RetryJob,
                on_bulk(&qid, q, 3),
                AuditOutcome::Failure
            ),
            row(
                AuditAction::RemoveJob,
                on_bulk(&qid, q, 2),
                AuditOutcome::Success
            ),
        ]
    );
}

pub async fn pausing_a_queue(backends: &dyn Backends, seeder: &dyn Seeder) {
    let q = "ranch-contract-pause";
    let conn = seeder.connection("contract pause", false).await;
    let qid = seeder.queue(&conn, q, &[("w1", Waiting)]).await;
    let backend = backends.open(&conn).await.expect("open");
    let control = backend.queue_control().expect("queue control");

    let paused = control
        .pause_queue(qid.clone(), Caller::User)
        .await
        .unwrap();
    assert!(paused.is_paused && !paused.already_in_state);
    assert!(queue_named(backend.as_ref(), &qid).await.is_paused);
    let again = control
        .pause_queue(qid.clone(), Caller::User)
        .await
        .unwrap();
    assert!(
        again.is_paused && again.already_in_state,
        "pausing twice is a no-op"
    );

    let resumed = control
        .resume_queue(qid.clone(), Caller::User)
        .await
        .unwrap();
    assert!(!resumed.is_paused && !resumed.already_in_state);
    assert!(!queue_named(backend.as_ref(), &qid).await.is_paused);
    let again = control
        .resume_queue(qid.clone(), Caller::User)
        .await
        .unwrap();
    assert!(!again.is_paused && again.already_in_state);
    let queue = queue_named(backend.as_ref(), &qid).await;
    assert_eq!(counts(&queue), [1, 0, 0, 0, 0], "the job is back in line");

    let pause = row(
        AuditAction::PauseQueue,
        on_queue(&qid, q),
        AuditOutcome::Success,
    );
    let resume = row(
        AuditAction::ResumeQueue,
        on_queue(&qid, q),
        AuditOutcome::Success,
    );
    assert_eq!(
        seeder.audit(&conn).await,
        [pause.clone(), pause, resume.clone(), resume]
    );
}

pub async fn maintenance(backends: &dyn Backends, seeder: &dyn Seeder) {
    let q = "ranch-contract-maintenance";
    let conn = seeder.connection("contract maintenance", false).await;
    let jobs = [
        ("w1", Waiting),
        ("w2", Waiting),
        ("d1", Delayed),
        ("c1", Completed),
        ("f1", Failed),
        ("f2", Failed),
    ];
    let qid = seeder.queue(&conn, q, &jobs).await;
    let backend = backends.open(&conn).await.expect("open");
    let maintenance = backend.maintenance().expect("maintenance");
    let drain = |include_delayed| QueueMaintenance::Drain { include_delayed };
    let clean_completed = QueueMaintenance::Clean {
        state: CleanState::Completed,
        older_than_ms: 0,
    };

    for (operation, affected) in [
        (drain(false), 2),
        (drain(true), 3),
        (clean_completed.clone(), 1),
        (QueueMaintenance::RetryAllFailed, 2),
    ] {
        let preview = maintenance
            .preview(qid.clone(), operation.clone())
            .await
            .unwrap();
        assert_eq!(preview.affected, affected, "preview of {operation:?}");
    }
    assert!(
        seeder.audit(&conn).await.is_empty(),
        "a preview changes nothing"
    );

    let retried = maintenance
        .run(
            qid.clone(),
            QueueMaintenance::RetryAllFailed,
            super::no_progress(),
            Caller::User,
        )
        .await
        .unwrap();
    assert_eq!((retried.action.as_str(), retried.affected), ("retry", 2));
    assert_eq!(
        counts(&queue_named(backend.as_ref(), &qid).await),
        [4, 0, 1, 0, 1]
    );

    let cleaned = maintenance
        .run(
            qid.clone(),
            clean_completed,
            super::no_progress(),
            Caller::User,
        )
        .await
        .unwrap();
    assert_eq!((cleaned.action.as_str(), cleaned.affected), ("clean", 1));

    let drained = maintenance
        .run(qid.clone(), drain(true), super::no_progress(), Caller::User)
        .await
        .unwrap();
    assert_eq!((drained.action.as_str(), drained.affected), ("drain", 5));
    assert!(!drained.cancelled);
    assert_eq!(
        counts(&queue_named(backend.as_ref(), &qid).await),
        [0, 0, 0, 0, 0]
    );

    assert_eq!(
        seeder.audit(&conn).await,
        [
            row(
                AuditAction::RetryAllFailed,
                on_queue(&qid, q),
                AuditOutcome::Success
            ),
            row(
                AuditAction::CleanQueue,
                on_queue(&qid, q),
                AuditOutcome::Success
            ),
            row(
                AuditAction::DrainQueue,
                on_queue(&qid, q),
                AuditOutcome::Success
            ),
        ]
    );
}

pub async fn schedules(backends: &dyn Backends, seeder: &dyn Seeder) {
    let q = "ranch-contract-schedules";
    let conn = seeder.connection("contract schedules", false).await;
    let qid = seeder.queue(&conn, q, &[]).await;
    let backend = backends.open(&conn).await.expect("open");
    let schedules = backend.schedules().expect("schedules");
    let key = |key: &str| SchedulerTarget {
        queue_id: qid.clone(),
        scheduler_key: key.into(),
    };
    let listed = || async { schedules.list_schedulers(qid.clone()).await.unwrap() };

    let saved = schedules
        .upsert_scheduler(
            qid.clone(),
            SchedulerSpec {
                id: "nightly".into(),
                job_name: "report".into(),
                schedule: Schedule::Pattern {
                    pattern: "0 3 * * *".into(),
                    tz: None,
                },
                data: "{}".into(),
            },
            Caller::User,
        )
        .await
        .unwrap();
    assert_eq!(saved.scheduler_id, "nightly");
    let cron = listed().await;
    assert_eq!(cron.len(), 1);
    assert_eq!(
        (cron[0].key.as_str(), cron[0].name.as_str()),
        ("nightly", "report")
    );
    assert_eq!(cron[0].pattern.as_deref(), Some("0 3 * * *"));
    assert!(!cron[0].paused);
    assert_eq!(cron[0].next_run, saved.next_run_ms);

    let paused = schedules
        .pause_scheduler(key("nightly"), Caller::User)
        .await
        .unwrap();
    assert!(paused.paused && !paused.already_in_state);
    assert!(listed().await[0].paused);
    let again = schedules
        .pause_scheduler(key("nightly"), Caller::User)
        .await
        .unwrap();
    assert!(again.paused && again.already_in_state);
    let resumed = schedules
        .resume_scheduler(key("nightly"), Caller::User)
        .await
        .unwrap();
    assert!(!resumed.paused && !resumed.already_in_state);
    assert!(!listed().await[0].paused);

    schedules
        .remove_scheduler(key("nightly"), Caller::User)
        .await
        .unwrap();
    assert!(listed().await.is_empty());
    let unknown = schedules
        .pause_scheduler(key("nightly"), Caller::User)
        .await;
    assert!(
        matches!(unknown, Err(RanchError::SchedulerNotFound { .. })),
        "a removed scheduler is not found: {unknown:?}"
    );

    let on = on_scheduler(&qid, q, "nightly");
    let success = |action| row(action, on.clone(), AuditOutcome::Success);
    assert_eq!(
        seeder.audit(&conn).await,
        [
            success(AuditAction::UpsertScheduler),
            success(AuditAction::PauseScheduler),
            success(AuditAction::PauseScheduler),
            success(AuditAction::ResumeScheduler),
            success(AuditAction::RemoveScheduler),
        ]
    );
}

pub async fn workers(backends: &dyn Backends, seeder: &dyn Seeder) {
    let q = "ranch-contract-workers";
    let conn = seeder.connection("contract workers", false).await;
    let qid = seeder.queue(&conn, q, &[("w1", Waiting)]).await;
    let backend = backends.open(&conn).await.expect("open");
    let workers = backend.workers().expect("workers");

    let listed = workers.list_workers(qid.clone()).await.unwrap();
    assert!(listed.supported);
    assert!(listed.workers.is_empty(), "nobody consumes the queue");
    let counted = workers.count_workers(vec![qid.clone()]).await.unwrap();
    assert!(counted.supported);
    assert_eq!(counted.counts.len(), 1);
    assert_eq!(
        (
            counted.counts[0].queue_id.clone(),
            counted.counts[0].workers
        ),
        (qid.clone(), 0)
    );
}

fn locked<T>(result: &Result<T, RanchError>) -> bool {
    matches!(result, Err(RanchError::ConnectionLocked { .. }))
}

pub async fn the_lock(backends: &dyn Backends, seeder: &dyn Seeder) {
    let q = "ranch-contract-locked";
    let conn = seeder.connection("contract locked", true).await;
    let qid = seeder.queue(&conn, q, &[("f1", Failed)]).await;
    let backend = backends.open(&conn).await.expect("open");
    let control = backend.connection();
    let actions = backend.job_actions().expect("job actions");

    assert_eq!(control.lock_state().await.unwrap(), ReadOnlyState::Locked);
    let retry = actions.retry_job(job(&qid, "f1"), Caller::User).await;
    assert!(
        locked(&retry),
        "a locked connection refuses a retry: {retry:?}"
    );
    let many = actions
        .bulk_retry_jobs(bulk(&qid, &["f1"]), super::no_progress(), Caller::User)
        .await;
    assert!(locked(&many), "and a bulk retry: {many:?}");
    let pause = backend
        .queue_control()
        .expect("queue control")
        .pause_queue(qid.clone(), Caller::User)
        .await;
    assert!(locked(&pause), "and a pause: {pause:?}");
    assert_eq!(
        state_of(backend.as_ref(), &qid, "f1").await,
        Failed,
        "reads still work, and nothing moved"
    );
    // Only the bulk refusal is recorded, as the attempt it was.
    assert_eq!(
        seeder.audit(&conn).await,
        [row(
            AuditAction::RetryJob,
            on_bulk(&qid, q, 1),
            AuditOutcome::Failure
        )]
    );

    assert_eq!(control.unlock().await.unwrap(), ReadOnlyState::Unlocked);
    actions
        .retry_job(job(&qid, "f1"), Caller::User)
        .await
        .unwrap();
    assert_eq!(control.lock().await.unwrap(), ReadOnlyState::Locked);
    let retry = actions.retry_job(job(&qid, "f1"), Caller::User).await;
    assert!(locked(&retry), "locked again: {retry:?}");
    assert_eq!(seeder.audit(&conn).await.len(), 2);
}
