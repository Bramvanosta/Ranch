//! Operations on the in-memory backend: the rules they keep whatever the
//! backend: the lock, approvals, the audit log, and refusing what the
//! backend cannot do.

mod common;
use common::no_progress;

use ranch_lib::adapters::bullmq_redis::bullmq_schema::JobState;
use ranch_lib::adapters::fake_backend::{FakeBackends, FakeJob};
use ranch_lib::commands::jobs::{
    bulk_retry_jobs_inner, retry_job_inner, BulkActionArgs, JobActionArgs,
};
use ranch_lib::commands::queues::{
    follow_connection_events_inner, get_connection_metrics_inner, get_queue_metrics_inner,
    get_server_info_inner, FollowConnectionEventsArgs, GetConnectionMetricsArgs,
    GetQueueMetricsArgs, GetServerInfoArgs,
};
use ranch_lib::domain::audit::ApprovalGrant;
use ranch_lib::domain::audit::{AuditOutcome, AuditSource, AuditTarget, Caller};
use ranch_lib::domain::errors::RanchError;
use ranch_lib::domain::lock::ReadOnlyState;
use ranch_lib::domain::queues::QueueId;

const CONN: &str = "prod";

fn emails() -> QueueId {
    FakeBackends::queue_id("emails")
}

/// A locked connection whose `emails` queue holds one failed job, `f1`.
async fn locked_with_a_failure() -> FakeBackends {
    let backends = FakeBackends::new();
    backends.add_connection(CONN, "Production", ReadOnlyState::Locked);
    let failed = FakeJob::new("f1", JobState::Failed, 1_700_000_000_000);
    backends.seed_queue(CONN, "emails", vec![failed]).await;
    backends
}

fn retry_f1() -> JobActionArgs {
    JobActionArgs {
        connection_id: CONN.into(),
        queue_id: emails().into(),
        job_id: "f1".into(),
    }
}

#[tokio::test]
async fn the_lock_refuses_a_retry_and_records_a_refused_bulk() {
    let backends = locked_with_a_failure().await;

    let single = retry_job_inner(retry_f1(), &backends, Caller::User).await;
    assert!(matches!(single, Err(RanchError::ConnectionLocked { .. })));
    assert!(
        backends.audit().is_empty(),
        "a single refusal is not recorded"
    );

    let many = BulkActionArgs {
        connection_id: CONN.into(),
        queue_id: emails().into(),
        job_ids: vec!["f1".into()],
    };
    let refused = bulk_retry_jobs_inner(many, &backends, no_progress(), Caller::User).await;
    assert!(matches!(refused, Err(RanchError::ConnectionLocked { .. })));
    let rows = backends.audit();
    assert_eq!(rows.len(), 1, "the bulk attempt is recorded once");
    assert_eq!(rows[0].outcome, AuditOutcome::Failure);
    assert_eq!(rows[0].outcome_reason.as_deref(), Some("Connection locked"));
    assert_eq!(
        rows[0].target,
        AuditTarget::bulk_jobs(&emails(), "emails", 1)
    );
}

#[tokio::test]
async fn an_approval_lets_an_agent_through_the_lock_once() {
    let backends = locked_with_a_failure().await;
    backends.approvals().approve("a1", CONN, "retry_job");
    let approved = || {
        Caller::agent("claude-desktop", "stdio")
            .with_approval(Some(ApprovalGrant::forge_for_test("a1", "retry_job")))
    };

    let retried = retry_job_inner(retry_f1(), &backends, approved())
        .await
        .unwrap();
    assert_eq!(retried.new_state, Some(JobState::Waiting.into()));
    let rows = backends.audit();
    assert_eq!(rows[0].source, AuditSource::Agent);
    assert_eq!(rows[0].approval_id.as_deref(), Some("a1"));
    assert_eq!(rows[0].connection_name.as_deref(), Some("Production"));

    let reused = retry_job_inner(retry_f1(), &backends, approved()).await;
    assert!(matches!(reused, Err(RanchError::ApprovalInvalid { .. })));
}

#[tokio::test]
async fn what_the_backend_cannot_do_is_refused_by_name() {
    let backends = locked_with_a_failure().await;
    let unsupported = |result: Result<_, RanchError>| match result {
        Err(RanchError::Unsupported { capability }) => capability,
        Ok(_) => panic!("expected Unsupported, got a result"),
        Err(other) => panic!("expected Unsupported, got {other:?}"),
    };

    let metrics = get_queue_metrics_inner(
        GetQueueMetricsArgs {
            connection_id: CONN.into(),
            queue_id: emails().into(),
            range: Default::default(),
            now_ms: None,
        },
        &backends,
    )
    .await;
    assert_eq!(unsupported(metrics.map(|_| ())), "metrics.queue");
    let overall = get_connection_metrics_inner(
        GetConnectionMetricsArgs {
            connection_id: CONN.into(),
            range: Default::default(),
        },
        &backends,
    )
    .await;
    assert_eq!(unsupported(overall.map(|_| ())), "metrics.queue");
    let server = get_server_info_inner(
        GetServerInfoArgs {
            connection_id: CONN.into(),
        },
        &backends,
    )
    .await;
    assert_eq!(unsupported(server.map(|_| ())), "server.info");

    let followed = follow_connection_events_inner(
        FollowConnectionEventsArgs {
            connection_id: CONN.into(),
            follow: true,
        },
        &backends,
        no_progress(),
    )
    .await;
    assert!(
        !followed.unwrap(),
        "without live events, the app keeps polling"
    );
}
