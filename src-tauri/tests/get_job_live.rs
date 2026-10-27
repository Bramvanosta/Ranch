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
//!   cargo test --test get_job_live -- --nocapture
//! ```

use ranch_lib::commands::jobs::{GetJobArgs, ListJobsArgs};
use ranch_lib::core::bullmq_schema::{JobState, QueueRef};
use ranch_lib::core::connection_actor::{ConnectionConfig, ConnectionRegistry};
use ranch_lib::core::errors::RanchErrorDto;

async fn local_redis_cfg() -> ConnectionConfig {
    ConnectionConfig {
        host: std::env::var("RANCH_REDIS_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
        port: std::env::var("RANCH_REDIS_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(6390),
        db: 0,
        username: None,
        password: None,
        tls: false,
        tls_custom_ca: None,
        ssh_tunnel: None,
        ssh_password: None,
        keychain: std::sync::Arc::new(ranch_lib::core::keychain::InMemoryKeychain::new()),
        initial_lock_state: ranch_lib::core::safety_lock::ReadOnlyState::Unlocked,
        connection_id: "test-conn".into(),
        connection_name: "test".into(),
        env_color: Some("env-local".into()),
        pool: sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory pool"),
    }
}

fn skip_if_no_local_redis() -> bool {
    if std::env::var("RANCH_LOCAL_REDIS").is_err() {
        eprintln!(
            "skipping: set RANCH_LOCAL_REDIS=1 (and optionally RANCH_REDIS_HOST/PORT) + run \
             `docker compose --profile redis --profile bullmq up -d`"
        );
        return true;
    }
    false
}

fn qref(queue: &str) -> QueueRef {
    QueueRef {
        prefix: "bull".into(),
        name: queue.into(),
    }
}

#[tokio::test]
async fn fetches_failed_email_with_full_decode() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;

    // First, list failed emails to pick one we know exists.
    let list = registry
        .list_jobs(
            "conn-get-job",
            &cfg,
            ListJobsArgs {
                connection_id: "conn-get-job".into(),
                queue_ref: qref("emails"),
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
            GetJobArgs {
                connection_id: "conn-get-job".into(),
                queue_ref: qref("emails"),
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
async fn fetches_completed_image_with_return_value() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;

    let list = registry
        .list_jobs(
            "conn-get-job-c",
            &cfg,
            ListJobsArgs {
                connection_id: "conn-get-job-c".into(),
                queue_ref: qref("images"),
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
            GetJobArgs {
                connection_id: "conn-get-job-c".into(),
                queue_ref: qref("images"),
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
async fn returns_job_not_found_for_bad_id() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;

    let result = registry
        .get_job(
            "conn-get-job-404",
            &cfg,
            GetJobArgs {
                connection_id: "conn-get-job-404".into(),
                queue_ref: qref("emails"),
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
