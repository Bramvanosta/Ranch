//! Live integration tests for `list_jobs` against the dockered Redis
//! seeded by `compose --profile bullmq up bullmq-seed`.
//!
//! Gated on `RANCH_LOCAL_REDIS=1`. Same skip-if-absent pattern as
//! the scaling soak and `list_queues_live`.
//!
//! ```bash
//! docker compose --profile redis --profile bullmq up -d
//! RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 \
//!   cargo test --test list_jobs_live -- --nocapture
//! ```
//!
//! The seed creates 3 queues with this rough shape after a single run:
//!   - emails  (50 wait + 10 delayed + 5 fail)
//!   - images  (8 done + 22 wait)
//!   - reports (5 wait)

use std::time::Instant;

use ranch_lib::commands::jobs::ListJobsArgs;
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
            "skipping: set RANCH_LOCAL_REDIS=1 (and optionally RANCH_REDIS_HOST/PORT) + run \
             `docker compose --profile redis --profile bullmq up -d`"
        );
        return true;
    }
    false
}

fn args_for(queue: &str, state: JobState, offset: u64, limit: u64) -> ListJobsArgs {
    ListJobsArgs {
        connection_id: format!("conn-{queue}-{state:?}").to_lowercase(),
        queue_ref: QueueRef {
            prefix: "bull".into(),
            name: queue.into(),
        },
        state,
        offset,
        limit,
        newest_first: true,
        search: None,
        reason: None,
        sort: None,
    }
}

#[tokio::test]
async fn lists_failed_emails_and_decodes_stacktraces() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;
    let out = registry
        .list_jobs(
            "conn-failed",
            &cfg,
            args_for("emails", JobState::Failed, 0, 50),
        )
        .await
        .expect("actor send")
        .expect("list_jobs");

    assert!(out.total >= 5, "expected total >= 5, got {}", out.total);
    assert!(!out.jobs.is_empty(), "expected at least one decoded job");
    for j in &out.jobs {
        assert_eq!(j.state, JobState::Failed);
        assert!(
            j.failed_reason.is_some(),
            "failed job missing failedReason: {:?}",
            j.job_ref
        );
        assert!(
            !j.stacktrace.is_empty(),
            "failed job missing stacktrace: {:?}",
            j.job_ref
        );
    }
    registry.shutdown_all().await;
}

#[tokio::test]
async fn lists_completed_images_and_decodes_return_value() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;
    let out = registry
        .list_jobs(
            "conn-completed",
            &cfg,
            args_for("images", JobState::Completed, 0, 50),
        )
        .await
        .expect("actor send")
        .expect("list_jobs");

    assert!(out.total >= 8, "expected total >= 8, got {}", out.total);
    for j in &out.jobs {
        assert_eq!(j.state, JobState::Completed);
        assert!(
            j.return_value.is_some(),
            "completed job missing returnValue: {:?}",
            j.job_ref
        );
        // returnValue is a JSON string; sanity-check it parses.
        let rv = j.return_value.as_ref().unwrap();
        let _: serde_json::Value =
            serde_json::from_str(rv).expect("returnValue should be parseable JSON");
    }
    registry.shutdown_all().await;
}

#[tokio::test]
async fn pagination_offsets_advance_correctly() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;

    let page0 = registry
        .list_jobs(
            "conn-page",
            &cfg,
            args_for("emails", JobState::Failed, 0, 2),
        )
        .await
        .expect("actor send")
        .expect("list_jobs");
    assert!(page0.jobs.len() <= 2);

    let page1 = registry
        .list_jobs(
            "conn-page",
            &cfg,
            args_for("emails", JobState::Failed, 2, 2),
        )
        .await
        .expect("actor send")
        .expect("list_jobs");
    assert!(page1.jobs.len() <= 2);

    // The two pages must contain different job IDs (pagination works).
    let ids0: std::collections::HashSet<&str> =
        page0.jobs.iter().map(|j| j.job_ref.id.as_str()).collect();
    let ids1: std::collections::HashSet<&str> =
        page1.jobs.iter().map(|j| j.job_ref.id.as_str()).collect();
    assert!(
        ids0.is_disjoint(&ids1),
        "pages overlap: {ids0:?} vs {ids1:?}"
    );

    registry.shutdown_all().await;
}

#[tokio::test]
async fn budget_for_50_job_page() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;

    // Cold call. The target is <100ms per page of 50 jobs on the baseline
    // machine. Docker overhead may push us slightly above; the soft
    // assertion below allows up to 250ms (still well within the 500ms
    // budget for the whole drill-into-queue flow).
    let start = Instant::now();
    let out = registry
        .list_jobs(
            "conn-budget",
            &cfg,
            args_for("emails", JobState::Waiting, 0, 50),
        )
        .await
        .expect("actor send")
        .expect("list_jobs");
    let elapsed = start.elapsed();
    eprintln!(
        "list_jobs(emails, Waiting, 0, 50): {elapsed:?}, returned {} of {}",
        out.jobs.len(),
        out.total
    );
    assert!(
        elapsed.as_millis() < 250,
        "budget breached (soft): {elapsed:?} (target: <100ms, soft cap 250ms)"
    );

    registry.shutdown_all().await;
}

#[tokio::test]
async fn empty_state_returns_empty_jobs_not_error() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;

    // `reports` queue has waiting jobs but no failures yet (per seed).
    // Querying its `Failed` state should return an empty list cleanly.
    let out = registry
        .list_jobs(
            "conn-empty",
            &cfg,
            args_for("reports", JobState::Failed, 0, 50),
        )
        .await
        .expect("actor send")
        .expect("list_jobs");

    assert_eq!(out.total, 0, "expected total=0, got {}", out.total);
    assert!(out.jobs.is_empty());
    assert!(!out.has_more);

    registry.shutdown_all().await;
}

#[tokio::test]
async fn stalled_state_returns_empty_on_v5_when_set_absent() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;

    // No real stalled jobs in the seed. Should not error — just return
    // an empty list. ZCARD on a missing key returns 0.
    let out = registry
        .list_jobs(
            "conn-stalled",
            &cfg,
            args_for("emails", JobState::Stalled, 0, 50),
        )
        .await
        .expect("actor send")
        .expect("list_jobs");
    assert_eq!(out.total, 0);
    assert!(out.jobs.is_empty());

    registry.shutdown_all().await;
}

#[tokio::test]
async fn unknown_state_returns_empty_not_error() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;
    let out = registry
        .list_jobs(
            "conn-unknown",
            &cfg,
            args_for("emails", JobState::Unknown, 0, 50),
        )
        .await
        .expect("actor send")
        .expect("list_jobs");
    assert_eq!(out.total, 0);
    assert!(out.jobs.is_empty());
    registry.shutdown_all().await;
}

#[tokio::test]
async fn has_more_flips_correctly_at_boundaries() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;

    // First small page → has_more should be true (there are >= 5 failed jobs).
    let small = registry
        .list_jobs("conn-hm", &cfg, args_for("emails", JobState::Failed, 0, 2))
        .await
        .expect("actor send")
        .expect("list_jobs");
    if small.total > 2 {
        assert!(
            small.has_more,
            "expected has_more=true with total>2 + page=2"
        );
    }

    // Page covering all → has_more should be false.
    let all = registry
        .list_jobs(
            "conn-hm",
            &cfg,
            args_for("emails", JobState::Failed, 0, 1000),
        )
        .await
        .expect("actor send")
        .expect("list_jobs");
    assert!(
        !all.has_more,
        "expected has_more=false when page covers all jobs"
    );

    registry.shutdown_all().await;
}
