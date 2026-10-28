//! Live integration tests for `get_job_logs` against the dockered Redis
//! seeded by `compose --profile redis --profile bullmq up bullmq-seed`
//!.
//!
//! The seed adds a single distinct failed job named `send-broken-with-logs`
//! in the `emails` queue that calls `Job.log()` three times before
//! throwing: the exact known content these tests assert against.
//!
//! Gated on `RANCH_LOCAL_REDIS=1`.

use ranch_lib::adapters::bullmq_redis::bullmq_schema::{JobState, QueueRef};
use ranch_lib::adapters::bullmq_redis::connection_actor::{ConnectionConfig, ConnectionRegistry};
use ranch_lib::domain::jobs::{JobLogsQuery, JobPageQuery};

mod common;
use common::redis_target;

fn qref(queue: &str) -> QueueRef {
    QueueRef {
        prefix: "bull".into(),
        name: queue.into(),
    }
}

/// Walk the failed jobs in `emails` and return the id of the one whose
/// `name` is `send-broken-with-logs` (the seeded log-test job).
async fn find_logged_failed_job_id(
    registry: &ConnectionRegistry,
    cfg: &ConnectionConfig,
) -> String {
    let list = registry
        .list_jobs(
            "conn-logs-discover",
            cfg,
            JobPageQuery {
                queue_id: qref("emails").id(),
                state: JobState::Failed,
                offset: 0,
                limit: 50,
                newest_first: true,
                search: None,
                reason: None,
                sort: None,
            },
        )
        .await
        .expect("actor send")
        .expect("list_jobs");
    list.jobs
        .into_iter()
        .find(|j| j.name == "send-broken-with-logs")
        .expect("expected seed to include a send-broken-with-logs failed job")
        .job_ref
        .id
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn returns_empty_for_jobs_without_logs() {
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;

    // Find any failed email job that is NOT our log-target; those
    // never call `Job.log()` so the `:logs` key won't exist.
    let list = registry
        .list_jobs(
            "conn-no-logs",
            &cfg,
            JobPageQuery {
                queue_id: qref("emails").id(),
                state: JobState::Failed,
                offset: 0,
                limit: 50,
                newest_first: true,
                search: None,
                reason: None,
                sort: None,
            },
        )
        .await
        .expect("actor send")
        .expect("list_jobs");
    let job = list
        .jobs
        .iter()
        .find(|j| j.name != "send-broken-with-logs")
        .expect("expected at least one failed email that isn't the log-target");
    let job_id = job.job_ref.id.clone();

    let out = registry
        .get_job_logs(
            "conn-no-logs",
            &cfg,
            JobLogsQuery {
                queue_id: qref("emails").id(),
                job_id,
                offset: 0,
                limit: 200,
            },
        )
        .await
        .expect("actor send")
        .expect("get_job_logs");

    assert_eq!(out.total, 0);
    assert!(out.lines.is_empty());
    assert!(!out.has_more);

    registry.shutdown_all().await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn returns_lines_for_seeded_logs_when_present() {
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;
    let job_id = find_logged_failed_job_id(&registry, &cfg).await;

    let out = registry
        .get_job_logs(
            "conn-with-logs",
            &cfg,
            JobLogsQuery {
                queue_id: qref("emails").id(),
                job_id,
                offset: 0,
                limit: 200,
            },
        )
        .await
        .expect("actor send")
        .expect("get_job_logs");

    assert_eq!(out.total, 3, "expected 3 log lines, got {}", out.total);
    assert_eq!(
        out.lines,
        vec![
            "Attempting send...".to_string(),
            "Network timeout".to_string(),
            "Will retry".to_string(),
        ]
    );
    assert!(!out.has_more);

    registry.shutdown_all().await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn pagination_works() {
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;
    let job_id = find_logged_failed_job_id(&registry, &cfg).await;

    let out = registry
        .get_job_logs(
            "conn-logs-paging",
            &cfg,
            JobLogsQuery {
                queue_id: qref("emails").id(),
                job_id,
                offset: 1,
                limit: 1,
            },
        )
        .await
        .expect("actor send")
        .expect("get_job_logs");

    assert_eq!(out.total, 3);
    assert_eq!(out.lines, vec!["Network timeout".to_string()]);
    assert!(out.has_more);

    registry.shutdown_all().await;
}
