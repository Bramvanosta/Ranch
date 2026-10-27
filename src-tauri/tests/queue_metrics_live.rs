//! Live tests for queue metrics.
//!
//! Gated on `RANCH_LOCAL_REDIS=1`, like the other `*_live.rs` suites:
//!
//!   docker compose --profile redis --profile bullmq up -d
//!   RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test queue_metrics_live
//!
//! The seed gives both code paths something to chew on:
//!   - `images` — its worker records BullMQ **native** metrics; 8 `resize`
//!     jobs complete.
//!   - `emails` — no native metrics → **estimated** counts; 5 `send-broken` +
//!     1 `send-broken-with-logs` fail.

use ranch_lib::commands::queues::GetQueueMetricsArgs;
use ranch_lib::core::bullmq_schema::{MetricsRange, QueueRef};
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

fn args(queue: &str, range: MetricsRange) -> GetQueueMetricsArgs {
    GetQueueMetricsArgs {
        connection_id: "test-conn".into(),
        queue_ref: QueueRef {
            prefix: "bull".into(),
            name: queue.into(),
        },
        range,
        now_ms: None,
    }
}

#[tokio::test]
async fn native_metrics_queue_reports_exact_counts_and_timings() {
    if skip() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;
    let m = registry
        .get_queue_metrics("m-native", &cfg, args("images", MetricsRange::D7))
        .await
        .expect("actor send")
        .expect("metrics");

    assert_eq!(
        (m.native_queues, m.total_queues),
        (1, 1),
        "images records native metrics"
    );
    assert!(
        m.completed_total >= 8,
        "8 resize jobs completed, got {}",
        m.completed_total
    );
    assert_eq!(m.completed_series.len(), 28);
    assert_eq!(m.completed_series.iter().sum::<u64>(), m.completed_total);
    assert_eq!(m.bucket_starts_ms.len(), 28);
    assert!(m.sampled_jobs >= 8);
    assert!(m.max_processing_time_ms >= m.min_processing_time_ms);

    assert!(!m.slowest_jobs.is_empty() && m.slowest_jobs.len() <= 10);
    assert!(m
        .slowest_jobs
        .iter()
        .all(|j| j.queue_name == "images" && j.name == "resize"));
    assert!(
        m.slowest_jobs
            .windows(2)
            .all(|w| w[0].processing_time_ms >= w[1].processing_time_ms),
        "slowest first"
    );
}

#[tokio::test]
async fn estimated_queue_reports_failing_job_types() {
    if skip() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;
    let m = registry
        .get_queue_metrics("m-estimated", &cfg, args("emails", MetricsRange::D7))
        .await
        .expect("actor send")
        .expect("metrics");

    assert_eq!(
        (m.native_queues, m.total_queues),
        (0, 1),
        "emails has no native metrics"
    );
    assert!(
        m.failed_total >= 6,
        "5 send-broken + 1 with logs, got {}",
        m.failed_total
    );
    assert!(m.failure_rate > 0.0);

    let broken = m
        .failing_job_types
        .iter()
        .find(|t| t.name == "send-broken")
        .expect("send-broken is a failing type");
    assert!(broken.count >= 5);
    assert_eq!(broken.queue_name, "emails");
    assert!(broken
        .last_reason
        .as_deref()
        .unwrap_or("")
        .contains("Connection refused"));
    assert!(m.failing_job_types[0].count >= m.failing_job_types.last().unwrap().count);
}

#[tokio::test]
async fn a_narrow_window_excludes_what_happened_before_it() {
    if skip() {
        return;
    }
    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;
    // Pretend "now" is a year from now: nothing of the seed is in the last 5 minutes.
    let mut a = args("emails", MetricsRange::M5);
    a.now_ms = Some((chrono::Utc::now().timestamp_millis() + 365 * 86_400_000) as f64);
    let m = registry
        .get_queue_metrics("m-window", &cfg, a)
        .await
        .expect("actor send")
        .expect("metrics");
    assert_eq!(m.completed_series, vec![0; 5]);
    assert_eq!(
        (m.completed_total, m.failed_total, m.sampled_jobs),
        (0, 0, 0)
    );
    assert!(m.failing_job_types.is_empty() && m.slowest_jobs.is_empty());
    assert_eq!(m.failure_rate, 0.0);
}
