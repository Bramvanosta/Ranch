//! Live integration tests for `list_cron_jobs` against the dockered
//! Redis seeded by `compose --profile redis --profile bullmq up
//! bullmq-seed`.
//!
//! The seed registers two schedulers on the `emails` queue:
//!   - `daily-report` — cron pattern `0 9 * * *` with `tz: Europe/Paris`.
//!   - `every-15m` — fixed interval `15 * 60 * 1000` ms.
//!
//! Gated on `RANCH_LOCAL_REDIS=1`, same skip-if-absent pattern as
//! the other live suites.

use std::time::Instant;

use ranch_lib::commands::queues::ListCronJobsArgs;
use ranch_lib::core::bullmq_schema::QueueRef;
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

#[tokio::test]
async fn lists_cron_and_every_schedulers_on_emails() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;

    let schedulers = registry
        .list_cron_jobs(
            "conn-crons",
            &cfg,
            ListCronJobsArgs {
                connection_id: "conn-crons".into(),
                queue_ref: qref("emails"),
            },
        )
        .await
        .expect("actor send")
        .expect("list_cron_jobs");

    assert!(
        schedulers.len() >= 2,
        "expected at least 2 schedulers, got {}",
        schedulers.len()
    );

    let daily = schedulers
        .iter()
        .find(|s| s.key == "daily-report")
        .expect("missing seeded `daily-report` scheduler");
    assert_eq!(daily.name, "send-daily-report");
    assert_eq!(daily.pattern.as_deref(), Some("0 9 * * *"));
    assert!(
        daily.every.is_none(),
        "cron scheduler should have no `every`"
    );
    assert_eq!(daily.tz.as_deref(), Some("Europe/Paris"));
    assert!(
        daily.next_run > 0,
        "next_run should be a future ms timestamp"
    );
    assert!(
        daily.template.is_some(),
        "seeded scheduler should carry a JSON template"
    );

    let every = schedulers
        .iter()
        .find(|s| s.key == "every-15m")
        .expect("missing seeded `every-15m` scheduler");
    assert_eq!(every.name, "ping-health");
    assert_eq!(every.every, Some(15 * 60 * 1000));
    assert!(
        every.pattern.is_none(),
        "fixed-interval scheduler should have no `pattern`"
    );
    assert!(every.tz.is_none(), "fixed-interval has no tz field");
    assert!(every.next_run > 0);

    registry.shutdown_all().await;
}

#[tokio::test]
async fn returns_empty_for_queue_without_schedulers() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;

    // The `images` queue has no schedulers in the seed.
    let schedulers = registry
        .list_cron_jobs(
            "conn-crons-empty",
            &cfg,
            ListCronJobsArgs {
                connection_id: "conn-crons-empty".into(),
                queue_ref: qref("images"),
            },
        )
        .await
        .expect("actor send")
        .expect("list_cron_jobs");

    assert!(
        schedulers.is_empty(),
        "expected empty Vec, got {schedulers:?}"
    );

    registry.shutdown_all().await;
}

#[tokio::test]
async fn per_call_within_budget() {
    if skip_if_no_local_redis() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;

    let start = Instant::now();
    let _ = registry
        .list_cron_jobs(
            "conn-crons-budget",
            &cfg,
            ListCronJobsArgs {
                connection_id: "conn-crons-budget".into(),
                queue_ref: qref("emails"),
            },
        )
        .await
        .expect("actor send")
        .expect("list_cron_jobs");
    let elapsed = start.elapsed();
    eprintln!("list_cron_jobs(emails): {elapsed:?}");
    // Soft cap mirrors the `list_jobs_live` budget test (docker overhead
    // budget). Realistic counts are ≤100 schedulers per queue; a
    // single pipelined HGETALL stays well under this.
    assert!(
        elapsed.as_millis() < 250,
        "soft budget breached: {elapsed:?} (target <250ms)"
    );

    registry.shutdown_all().await;
}
