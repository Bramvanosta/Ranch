//! Integration tests for `list_audit_log_entries` and
//! `export_audit_log`. Tests bypass the Tauri command boundary by
//! calling the inner functions directly with a `SqlitePool`.

mod common;
use common::repo;

use ranch_lib::adapters::sqlite::audit::record;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::commands::audit::{
    export_audit_log_inner, list_audit_log_entries_inner, ExportAuditLogArgs,
    ListAuditLogEntriesArgs,
};
use ranch_lib::contract::audit as wire;
use ranch_lib::contract::audit::AuditLogEntry;
use ranch_lib::domain::audit::{AuditAction, AuditEntry, AuditOutcome, AuditSource, AuditTarget};
use tempfile::tempdir;

async fn fresh_db() -> (tempfile::TempDir, Db) {
    let dir = tempdir().expect("tempdir");
    let db = Db::init(dir.path()).await.expect("Db::init");
    (dir, db)
}

fn entry(seq: usize, connection_id: &str, action: AuditAction, source: AuditSource) -> AuditEntry {
    AuditEntry {
        source,
        action,
        target: AuditTarget::Job {
            queue_id: None,
            queue_name: "emails".into(),
            job_id: format!("job-{seq}"),
        },
        outcome: AuditOutcome::Success,
        outcome_reason: None,
        connection_id: Some(connection_id.into()),
        connection_name: Some(format!("Acme · {connection_id}")),
        env_color: Some("env-production".into()),
        agent_client: None,
        agent_transport: None,
        approval_id: None,
    }
}

async fn seed(db: &Db) {
    // 10 entries: 5 on conn-a (RetryJob), 5 on conn-b (RemoveJob).
    // All source=ui for simplicity.
    for i in 0..5 {
        record(
            db.pool(),
            entry(i, "conn-a", AuditAction::RetryJob, AuditSource::Ui),
        )
        .await
        .unwrap();
    }
    for i in 5..10 {
        record(
            db.pool(),
            entry(i, "conn-b", AuditAction::RemoveJob, AuditSource::Ui),
        )
        .await
        .unwrap();
    }
}

fn args_for(
    connection_id: Option<&str>,
    action: Option<&str>,
    source: Option<&str>,
) -> ListAuditLogEntriesArgs {
    ListAuditLogEntriesArgs {
        from: None,
        to: None,
        connection_id: connection_id.map(String::from),
        action: action.map(String::from),
        source: source.map(String::from),
        offset: 0,
        limit: 200,
    }
}

#[tokio::test]
async fn lists_entries_newest_first() {
    let (_dir, db) = fresh_db().await;
    seed(&db).await;
    let entries = list_audit_log_entries_inner(args_for(None, None, None), db.pool())
        .await
        .unwrap();
    assert_eq!(entries.len(), 10);
    // Newest first by created_at + id.
    let timestamps: Vec<&str> = entries.iter().map(|e| e.created_at.as_str()).collect();
    let mut sorted_desc = timestamps.clone();
    sorted_desc.sort_by(|a, b| b.cmp(a));
    assert_eq!(timestamps, sorted_desc, "entries must be newest-first");
}

#[tokio::test]
async fn filters_by_connection_id() {
    let (_dir, db) = fresh_db().await;
    seed(&db).await;
    let entries = list_audit_log_entries_inner(args_for(Some("conn-a"), None, None), db.pool())
        .await
        .unwrap();
    assert_eq!(entries.len(), 5);
    assert!(entries
        .iter()
        .all(|e| e.connection_id.as_deref() == Some("conn-a")));
}

#[tokio::test]
async fn filters_by_action() {
    let (_dir, db) = fresh_db().await;
    seed(&db).await;
    let entries = list_audit_log_entries_inner(args_for(None, Some("retryJob"), None), db.pool())
        .await
        .unwrap();
    assert_eq!(entries.len(), 5);
    assert!(entries
        .iter()
        .all(|e| matches!(e.action, wire::AuditAction::RetryJob)));
}

#[tokio::test]
async fn filters_by_source() {
    let (_dir, db) = fresh_db().await;
    seed(&db).await;
    // Add one agent-source entry.
    record(
        db.pool(),
        AuditEntry {
            source: AuditSource::Agent,
            action: AuditAction::PauseQueue,
            target: AuditTarget::Queue {
                queue_id: None,
                queue_name: "emails".into(),
            },
            outcome: AuditOutcome::Success,
            outcome_reason: None,
            connection_id: Some("conn-a".into()),
            connection_name: Some("Acme · conn-a".into()),
            env_color: Some("env-production".into()),
            agent_client: Some("claude-desktop".into()),
            agent_transport: Some("stdio".into()),
            approval_id: None,
        },
    )
    .await
    .unwrap();

    let agents = list_audit_log_entries_inner(args_for(None, None, Some("agent")), db.pool())
        .await
        .unwrap();
    assert_eq!(agents.len(), 1);
    assert!(matches!(agents[0].source, wire::AuditSource::Agent));
    assert_eq!(agents[0].operator_id, "claude-desktop");

    let uis = list_audit_log_entries_inner(args_for(None, None, Some("ui")), db.pool())
        .await
        .unwrap();
    assert_eq!(uis.len(), 10);
}

#[tokio::test]
async fn pagination_offsets_advance_correctly() {
    let (_dir, db) = fresh_db().await;
    seed(&db).await;

    let page0 = list_audit_log_entries_inner(
        ListAuditLogEntriesArgs {
            from: None,
            to: None,
            connection_id: None,
            action: None,
            source: None,
            offset: 0,
            limit: 5,
        },
        db.pool(),
    )
    .await
    .unwrap();
    let page1 = list_audit_log_entries_inner(
        ListAuditLogEntriesArgs {
            from: None,
            to: None,
            connection_id: None,
            action: None,
            source: None,
            offset: 5,
            limit: 5,
        },
        db.pool(),
    )
    .await
    .unwrap();
    assert_eq!(page0.len(), 5);
    assert_eq!(page1.len(), 5);
    let ids0: std::collections::HashSet<&str> = page0.iter().map(|e| e.id.as_str()).collect();
    let ids1: std::collections::HashSet<&str> = page1.iter().map(|e| e.id.as_str()).collect();
    assert!(ids0.is_disjoint(&ids1), "pages must not overlap");
}

#[tokio::test]
async fn exports_to_json() {
    let (_dir, db) = fresh_db().await;
    seed(&db).await;
    let result = export_audit_log_inner(
        ExportAuditLogArgs {
            from: None,
            to: None,
            connection_id: None,
            action: None,
            source: None,
            format: "json".into(),
        },
        db.pool(),
    )
    .await
    .unwrap();

    let entries: Vec<AuditLogEntry> = serde_json::from_str(&result.content).unwrap();
    assert_eq!(entries.len(), 10);
    assert!(result.suggested_filename.ends_with(".json"));
    assert!(result.suggested_filename.starts_with("ranch-audit-"));
}

#[tokio::test]
async fn exports_to_csv() {
    let (_dir, db) = fresh_db().await;
    seed(&db).await;
    let result = export_audit_log_inner(
        ExportAuditLogArgs {
            from: None,
            to: None,
            connection_id: None,
            action: None,
            source: None,
            format: "csv".into(),
        },
        db.pool(),
    )
    .await
    .unwrap();

    let lines: Vec<&str> = result.content.lines().collect();
    // 1 header + 10 data rows.
    assert_eq!(lines.len(), 11);
    assert!(lines[0].contains("created_at,source,operator_id,action"));
    assert!(result.suggested_filename.ends_with(".csv"));
}

#[tokio::test]
async fn export_rejects_unknown_format() {
    let (_dir, db) = fresh_db().await;
    seed(&db).await;
    let err = export_audit_log_inner(
        ExportAuditLogArgs {
            from: None,
            to: None,
            connection_id: None,
            action: None,
            source: None,
            format: "yaml".into(),
        },
        db.pool(),
    )
    .await
    .unwrap_err();
    let dto: ranch_lib::contract::errors::RanchErrorDto = err.into();
    assert!(dto.message.contains("Unsupported"));
}

/// The listing joins the connection's group and
/// environment, so three connections that share a name can be told apart.
#[tokio::test]
async fn listing_carries_the_group_and_environment_of_the_connection() {
    use ranch_lib::adapters::secrets::InMemoryKeychain;
    use ranch_lib::application::ports::SecretStore;
    use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
    use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
    use std::sync::Arc;

    let (_dir, db) = fresh_db().await;
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "Lumen Studios".into(),
            backend: "bullmq-redis".into(),
            endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
                host: "127.0.0.1".into(),
                port: 6379,
                db: 0,
                username: None,
                tls: false,
                tls_custom_ca: None,
                ssh_tunnel: None,
            }
            .json(),
            password: None,
            ssh_password: None,
            group: GroupSelector::named("Lumen Studio"),
            environment: EnvironmentSelector::named("Production", "env-production"),
        },
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();

    record(
        db.pool(),
        entry(1, &conn.id, AuditAction::RemoveJob, AuditSource::Agent),
    )
    .await
    .unwrap();
    // A row whose connection no longer exists keeps its name snapshot and
    // simply has no group / environment to show.
    record(
        db.pool(),
        entry(2, "gone-for-good", AuditAction::RemoveJob, AuditSource::Ui),
    )
    .await
    .unwrap();

    let entries: Vec<AuditLogEntry> =
        list_audit_log_entries_inner(args_for(Some(&conn.id), None, None), db.pool())
            .await
            .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].group_name.as_deref(), Some("Lumen Studio"));
    assert_eq!(entries[0].environment_name.as_deref(), Some("Production"));

    let orphan: Vec<AuditLogEntry> =
        list_audit_log_entries_inner(args_for(Some("gone-for-good"), None, None), db.pool())
            .await
            .unwrap();
    assert_eq!(orphan.len(), 1);
    assert_eq!(orphan[0].group_name, None);
    assert_eq!(orphan[0].environment_name, None);
    assert_eq!(
        orphan[0].connection_name.as_deref(),
        Some("Acme · gone-for-good")
    );
}
