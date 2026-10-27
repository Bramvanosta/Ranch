//! Live integration tests for `get_job_logs` against the dockered Redis
//! seeded by `compose --profile redis --profile bullmq up bullmq-seed`
//!.
//!
//! The seed adds a single distinct failed job named `send-broken-with-logs`
//! in the `emails` queue that calls `Job.log()` three times before
//! throwing — the exact known content these tests assert against.
//!
//! Gated on `RANCH_LOCAL_REDIS=1`.

use ranch_lib::commands::jobs::{GetJobLogsArgs, ListJobsArgs};
use ranch_lib::core::bullmq_schema::{JobState, QueueRef};
use ranch_lib::core::connection_actor::{ConnectionConfig, ConnectionRegistry};

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
            "skipping: set RANCH_LOCAL_REDIS=1 + run \
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
            ListJobsArgs {
                connection_id: "conn-logs-discover".into(),
                queue_ref: qref("emails"),
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
async fn returns_empty_for_jobs_without_logs() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;

    // Find any failed email job that is NOT our log-target — those
    // never call `Job.log()` so the `:logs` key won't exist.
    let list = registry
        .list_jobs(
            "conn-no-logs",
            &cfg,
            ListJobsArgs {
                connection_id: "conn-no-logs".into(),
                queue_ref: qref("emails"),
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
            GetJobLogsArgs {
                connection_id: "conn-no-logs".into(),
                queue_ref: qref("emails"),
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
async fn returns_lines_for_seeded_logs_when_present() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;
    let job_id = find_logged_failed_job_id(&registry, &cfg).await;

    let out = registry
        .get_job_logs(
            "conn-with-logs",
            &cfg,
            GetJobLogsArgs {
                connection_id: "conn-with-logs".into(),
                queue_ref: qref("emails"),
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
async fn pagination_works() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;
    let job_id = find_logged_failed_job_id(&registry, &cfg).await;

    let out = registry
        .get_job_logs(
            "conn-logs-paging",
            &cfg,
            GetJobLogsArgs {
                connection_id: "conn-logs-paging".into(),
                queue_ref: qref("emails"),
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
