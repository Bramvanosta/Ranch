//! Live tests for the cross-queue failed view.
//!
//!   docker compose --profile redis --profile bullmq up -d
//!   RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test failed_jobs_live
//!
//! Seeded failures: `emails` — 5 `send-broken` ("Connection refused") + 1
//! `send-broken-with-logs` ("Network timeout"); `images` — the `resize` jobs
//! the seed worker refuses after its quota ("seeder-skip"). Other live suites
//! retry / remove jobs, so assertions are lower bounds and relations, not
//! exact counts.

use ranch_lib::commands::jobs::{ListFailedJobsArgs, ListFailureCausesArgs};
use ranch_lib::core::bullmq_schema::JobState;
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

fn skip() -> bool {
    if std::env::var("RANCH_LOCAL_REDIS").is_err() {
        eprintln!("skipping: set RANCH_LOCAL_REDIS=1 (see the module doc)");
        return true;
    }
    false
}

fn args(limit: u32) -> ListFailedJobsArgs {
    ListFailedJobsArgs {
        connection_id: "test-conn".into(),
        offset: 0,
        limit,
        ..Default::default()
    }
}

#[tokio::test]
async fn lists_failures_of_every_queue_newest_first() {
    if skip() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;
    let page = registry
        .list_failed_jobs("f-all", &cfg, args(250))
        .await
        .expect("actor send")
        .expect("page");

    assert!(
        page.total_failed >= 2,
        "seed leaves failures in emails and images"
    );
    assert_eq!(
        page.total_failed,
        page.queues.iter().map(|q| q.failed).sum::<u32>(),
        "total = sum of the per-queue counts"
    );
    assert!(
        page.queues.windows(2).all(|w| w[0].failed >= w[1].failed),
        "most failing first"
    );
    assert!(page.unreadable_queues.is_empty());

    assert!(!page.jobs.is_empty());
    assert!(page.jobs.iter().all(|j| j.state == JobState::Failed));
    assert!(
        page.jobs
            .windows(2)
            .all(|w| w[0].finished_on.unwrap_or(0) >= w[1].finished_on.unwrap_or(0)),
        "newest failure first, across queues"
    );
    let distinct_queues: std::collections::HashSet<&str> = page
        .jobs
        .iter()
        .map(|j| j.job_ref.queue.name.as_str())
        .collect();
    assert!(
        distinct_queues.len() >= 2,
        "one view, several queues: {distinct_queues:?}"
    );
    assert_eq!(
        page.total as usize,
        page.jobs.len(),
        "everything fits in one page of 250 here"
    );
    assert!(!page.types.is_empty());
}

#[tokio::test]
async fn filters_narrow_the_list_and_pagination_follows() {
    if skip() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;

    let mut only_emails = args(250);
    only_emails.queue = Some("emails".into());
    let emails = registry
        .list_failed_jobs("f-filter", &cfg, only_emails)
        .await
        .unwrap()
        .unwrap();
    assert!(emails.jobs.iter().all(|j| j.job_ref.queue.name == "emails"));
    assert!(emails.types.iter().all(|t| t.queue_name == "emails"));
    // The per-queue counts still describe the whole connection.
    assert!(emails.total_failed >= emails.total);

    let mut by_reason = args(250);
    by_reason.reason = Some("NETWORK TIMEOUT".into());
    let timeouts = registry
        .list_failed_jobs("f-filter", &cfg, by_reason)
        .await
        .unwrap()
        .unwrap();
    assert!(timeouts.jobs.iter().all(|j| j
        .failed_reason
        .as_deref()
        .unwrap_or("")
        .to_lowercase()
        .contains("network timeout")));

    // Two pages of one never overlap and keep the order.
    let first = registry
        .list_failed_jobs("f-filter", &cfg, args(1))
        .await
        .unwrap()
        .unwrap();
    let mut second_args = args(1);
    second_args.offset = 1;
    let second = registry
        .list_failed_jobs("f-filter", &cfg, second_args)
        .await
        .unwrap()
        .unwrap();
    if first.total >= 2 {
        assert_ne!(first.jobs[0].job_ref, second.jobs[0].job_ref);
        assert!(first.jobs[0].finished_on >= second.jobs[0].finished_on);
    }

    let mut nothing = args(50);
    nothing.name = Some("no-such-job-name".into());
    let empty = registry
        .list_failed_jobs("f-filter", &cfg, nothing)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((empty.total, empty.jobs.len()), (0, 0));
    assert!(empty.total_failed > 0, "the connection still has failures");
}

// The same failures, grouped by cause.
#[tokio::test]
async fn causes_group_the_failures_and_account_for_every_scanned_job() {
    if skip() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;
    let all = registry
        .list_failure_causes(
            "f-causes",
            &cfg,
            ListFailureCausesArgs {
                connection_id: "test-conn".into(),
                queue: None,
            },
        )
        .await
        .expect("actor send")
        .expect("causes");
    assert!(!all.causes.is_empty());
    assert_eq!(
        all.causes.iter().map(|c| c.count).sum::<u32>(),
        all.scanned,
        "every scanned failure is in exactly one cause"
    );
    assert!(
        all.causes.windows(2).all(|w| w[0].count >= w[1].count),
        "most frequent first"
    );
    for cause in &all.causes {
        assert_eq!(cause.jobs.len() as u32, cause.count);
        assert_eq!(cause.trend.len(), 24);
        assert!(cause.first_at <= cause.last_at);
    }

    let emails = registry
        .list_failure_causes(
            "f-causes",
            &cfg,
            ListFailureCausesArgs {
                connection_id: "test-conn".into(),
                queue: Some("emails".into()),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(emails
        .causes
        .iter()
        .all(|c| c.queue_names == vec!["emails".to_string()]));

    // A cause's signature filters the failed list down to that cause.
    let top = &emails.causes[0];
    let mut by_cause = args(250);
    by_cause.queue = Some("emails".into());
    by_cause.signature = Some(top.signature.clone());
    let page = registry
        .list_failed_jobs("f-causes", &cfg, by_cause)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(page.total, top.count);
}
