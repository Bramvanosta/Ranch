//! Live integration tests for `get_job` against the dockered Redis
//! seeded by `compose --profile redis --profile bullmq up bullmq-seed`
//!.
//!
//! Gated on `RANCH_LOCAL_REDIS=1`. Same skip-if-absent pattern as
//! the other live suites.
//!
//! ```bash
//! docker compose --profile redis --profile bullmq up -d
//! RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 \
//!   cargo test --test get_job_live -- --include-ignored --nocapture
//! ```

use ranch_lib::adapters::bullmq_redis::bullmq_schema::{JobState, QueueRef};
use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::contract::errors::RanchErrorDto;
use ranch_lib::domain::jobs::{JobPageQuery, JobTarget};

mod common;
use common::redis_target;

fn qref(queue: &str) -> QueueRef {
    QueueRef {
        prefix: "bull".into(),
        name: queue.into(),
    }
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn fetches_failed_email_with_full_decode() {
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;

    // First, list failed emails to pick one we know exists.
    let list = registry
        .list_jobs(
            "conn-get-job",
            &cfg,
            JobPageQuery {
                queue_id: qref("emails").id(),
                state: JobState::Failed,
                offset: 0,
                limit: 1,
                newest_first: true,
                search: None,
                reason: None,
                sort: None,
            },
        )
        .await
        .expect("actor send")
        .expect("list_jobs");
    let first = list
        .jobs
        .first()
        .expect("expected at least one failed email in the seed");
    let job_id = first.job_ref.id.clone();

    // Now fetch that single job via get_job.
    let job = registry
        .get_job(
            "conn-get-job",
            &cfg,
            JobTarget {
                queue_id: qref("emails").id(),
                job_id: job_id.clone(),
            },
        )
        .await
        .expect("actor send")
        .expect("get_job");

    assert_eq!(job.job_ref.id, job_id);
    assert_eq!(job.state, JobState::Failed);
    assert!(
        job.failed_reason.is_some(),
        "failed job missing failedReason: {:?}",
        job.job_ref
    );
    assert!(
        !job.stacktrace.is_empty(),
        "failed job missing stacktrace: {:?}",
        job.job_ref
    );
    // Data + opts came through as raw JSON strings.
    let _: serde_json::Value =
        serde_json::from_str(&job.data).expect("data should be parseable JSON");
    let _: serde_json::Value =
        serde_json::from_str(&job.opts).expect("opts should be parseable JSON");

    registry.shutdown_all().await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn fetches_completed_image_with_return_value() {
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;

    let list = registry
        .list_jobs(
            "conn-get-job-c",
            &cfg,
            JobPageQuery {
                queue_id: qref("images").id(),
                state: JobState::Completed,
                offset: 0,
                limit: 1,
                newest_first: true,
                search: None,
                reason: None,
                sort: None,
            },
        )
        .await
        .expect("actor send")
        .expect("list_jobs");
    let first = list
        .jobs
        .first()
        .expect("expected at least one completed image in the seed");
    let job_id = first.job_ref.id.clone();

    let job = registry
        .get_job(
            "conn-get-job-c",
            &cfg,
            JobTarget {
                queue_id: qref("images").id(),
                job_id: job_id.clone(),
            },
        )
        .await
        .expect("actor send")
        .expect("get_job");

    assert_eq!(job.state, JobState::Completed);
    assert!(
        job.return_value.is_some(),
        "completed job missing returnValue"
    );
    assert!(
        job.finished_on.is_some(),
        "completed job missing finishedOn"
    );
    let _: serde_json::Value = serde_json::from_str(job.return_value.as_ref().unwrap())
        .expect("returnValue should be parseable JSON");

    registry.shutdown_all().await;
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn returns_job_not_found_for_bad_id() {
    let registry = ConnectionRegistry::new();
    let cfg = redis_target::connection_config().await;

    let result = registry
        .get_job(
            "conn-get-job-404",
            &cfg,
            JobTarget {
                queue_id: qref("emails").id(),
                job_id: "999999999".into(),
            },
        )
        .await
        .expect("actor send");

    let err = result.expect_err("expected JobNotFound for a missing id");
    let dto: RanchErrorDto = err.into();
    assert_eq!(dto.kind, "jobNotFound", "got dto: {dto:?}");
    assert!(dto.message.contains("emails"));
    assert!(dto.message.contains("999999999"));

    registry.shutdown_all().await;
}
