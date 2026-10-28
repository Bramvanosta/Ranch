//! What decides an agent's rights carries the app's seal, and ranch-mcp
//! ignores what another process of the user wrote into `ranch.db`: an
//! "approved" status, a connection opened to agent writes, approvals turned
//! off, AI turned back on.
//!
//! The stores here check seals as ranch-mcp's do (`HmacSealer::for_agents`);
//! the app's writes seal with the same key, as they would in the GUI.

use std::sync::Arc;
use std::time::Duration;

use ranch_lib::adapters::hmac_sealer::HmacSealer;
use ranch_lib::adapters::sqlite::{SqliteConnectionRepository, SqliteSettingsStore};
use ranch_lib::adapters::system_clock::SystemClock;
use ranch_lib::application::ports::{ConnectionRepository, Sealer, SettingsStore};

use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::mcp_safety::set_connection_mcp_readonly_inner;
use ranch_lib::commands::settings::set_kv_setting_inner;
use ranch_lib::core::mcp::approval::{self, ApprovalDecision, ApprovalMode};
use ranch_lib::core::mcp::gate::{self, GateRequest, ToolKind};
use ranch_lib::core::mcp::heartbeat::write_heartbeat;
use ranch_lib::domain::errors::RanchError;
use sqlx::SqlitePool;
use tempfile::tempdir;

struct Env {
    _dir: tempfile::TempDir,
    pool: SqlitePool,
    sealer: Arc<HmacSealer>,
    repo: SqliteConnectionRepository,
    kv: SqliteSettingsStore,
    connection_id: String,
}

/// A staging connection open to agent writes, everything sealed as the first
/// run with a key leaves it.
async fn env() -> Env {
    let sealer = Arc::new(HmacSealer::for_agents(Some([42u8; 32])));
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let clock = Arc::new(SystemClock);
    let repo = SqliteConnectionRepository::new(db.pool().clone(), sealer.clone(), clock.clone());
    let kv = SqliteSettingsStore::new(db.pool().clone(), sealer.clone(), clock);
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "queues".into(),
            backend: "bullmq-redis".into(),
            endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
                host: "127.0.0.1".into(),
                port: 1,
                db: 0,
                username: None,
                tls: false,
                tls_custom_ca: None,
                ssh_tunnel: None,
            }
            .json(),
            password: None,
            ssh_password: None,
            group: GroupSelector::named("Lumen"),
            environment: EnvironmentSelector::named("Staging", "env-staging"),
        },
        &repo,
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    kv.seal_all().await.unwrap();
    repo.reseal_all().await.unwrap();
    Env {
        _dir: dir,
        pool: db.pool().clone(),
        sealer,
        repo,
        kv,
        connection_id: conn.id,
    }
}

fn sealer(env: &Env) -> &dyn Sealer {
    env.sealer.as_ref()
}

/// The app's side of the approval queue, for a task of its own: it lists
/// what waits and seals what it approves.
fn app(env: &Env) -> (SqlitePool, SqliteSettingsStore, Arc<HmacSealer>) {
    let kv = SqliteSettingsStore::new(env.pool.clone(), env.sealer.clone(), Arc::new(SystemClock));
    (env.pool.clone(), kv, env.sealer.clone())
}

fn request(env: &Env) -> GateRequest {
    GateRequest {
        tool: "remove_job".into(),
        kind: ToolKind::Write,
        connection_id: env.connection_id.clone(),
        client_hint: Some("claude-code".into()),
        arguments: serde_json::json!({ "jobId": "42" }),
        target_preview: None,
    }
}

async fn writable(env: &Env) -> bool {
    gate::check_mcp_readonly(
        &env.kv,
        &env.repo,
        &env.connection_id,
        "remove_job",
        ToolKind::Write,
    )
    .await
    .is_ok()
}

#[tokio::test]
async fn a_connection_opened_behind_the_apps_back_stays_read_only() {
    let e = env().await;
    assert!(writable(&e).await, "sealed by the app: open to writes");

    // The app closes it; another process opens it again.
    set_connection_mcp_readonly_inner(&e.connection_id, Some(true), &e.repo)
        .await
        .unwrap();
    assert!(!writable(&e).await);
    sqlx::query("UPDATE connections SET mcp_readonly = 0 WHERE id = ?")
        .bind(&e.connection_id)
        .execute(&e.pool)
        .await
        .unwrap();
    assert!(
        !writable(&e).await,
        "a value the app did not seal opens nothing"
    );

    // The app opens it: sealed, writable.
    set_connection_mcp_readonly_inner(&e.connection_id, Some(false), &e.repo)
        .await
        .unwrap();
    assert!(writable(&e).await);

    // Unlocking it in the file (the actor reads the stored lock) breaks the seal too.
    sqlx::query("UPDATE connections SET is_locked = 1 - is_locked WHERE id = ?")
        .bind(&e.connection_id)
        .execute(&e.pool)
        .await
        .unwrap();
    assert!(!writable(&e).await);
}

#[tokio::test]
async fn safety_settings_written_elsewhere_fall_back_to_the_strict_value() {
    let e = env().await;
    // Unset and sealed: the defaults.
    assert_eq!(
        approval::approval_mode(&e.kv).await.unwrap(),
        ApprovalMode::WritesOnly
    );
    assert!(gate::ai_enabled(&e.kv).await);

    // The app turns approvals off: honoured.
    set_kv_setting_inner(&e.kv, approval::APPROVAL_MODE_KEY, "off")
        .await
        .unwrap();
    assert_eq!(
        approval::approval_mode(&e.kv).await.unwrap(),
        ApprovalMode::Off
    );

    // The app turns them back on; another process turns them off again.
    set_kv_setting_inner(&e.kv, approval::APPROVAL_MODE_KEY, "all")
        .await
        .unwrap();
    sqlx::query("UPDATE kv_settings SET value = 'off' WHERE key = ?")
        .bind(approval::APPROVAL_MODE_KEY)
        .execute(&e.pool)
        .await
        .unwrap();
    assert_eq!(
        approval::approval_mode(&e.kv).await.unwrap(),
        ApprovalMode::WritesOnly
    );

    // AI off in the app, back on in the file: off. Deleted: off.
    set_kv_setting_inner(&e.kv, gate::AI_ENABLED_KEY, "false")
        .await
        .unwrap();
    sqlx::query("UPDATE kv_settings SET value = 'true' WHERE key = ?")
        .bind(gate::AI_ENABLED_KEY)
        .execute(&e.pool)
        .await
        .unwrap();
    assert!(!gate::ai_enabled(&e.kv).await);
    sqlx::query("DELETE FROM kv_settings WHERE key = ?")
        .bind(gate::AI_ENABLED_KEY)
        .execute(&e.pool)
        .await
        .unwrap();
    assert!(!gate::ai_enabled(&e.kv).await);

    // The app cannot be asked to write a seal itself.
    assert!(set_kv_setting_inner(&e.kv, "seal:ai_enabled", "x")
        .await
        .is_err());
}

#[tokio::test]
async fn an_approved_status_written_by_another_process_runs_nothing() {
    let e = env().await;
    write_heartbeat(&e.kv).await.unwrap();
    let forger = e.pool.clone();
    tokio::spawn(async move {
        loop {
            let n = sqlx::query(
                "UPDATE mcp_pending_approvals SET status = 'approved' WHERE status = 'pending'",
            )
            .execute(&forger)
            .await
            .unwrap()
            .rows_affected();
            if n > 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let err = gate::request_approval(&e.pool, &e.kv, sealer(&e), request(&e))
        .await
        .unwrap_err();
    assert!(matches!(err, RanchError::ApprovalDenied { .. }), "{err:?}");
}

#[tokio::test]
async fn the_apps_approval_is_honoured_and_bound_to_the_call() {
    let e = env().await;
    write_heartbeat(&e.kv).await.unwrap();

    // The app approves what it shows: the grant comes through.
    let (gui, kv, seals) = app(&e);
    tokio::spawn(async move {
        loop {
            if let Some(first) = approval::list_pending(&gui, &kv).await.unwrap().first() {
                approval::decide(&gui, &*seals, &first.id, ApprovalDecision::Approve, None)
                    .await
                    .unwrap();
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    gate::request_approval(&e.pool, &e.kv, sealer(&e), request(&e))
        .await
        .unwrap();

    // The arguments are swapped in the file before the human approves: the
    // seal covers what the app showed, not the call ranch-mcp holds.
    let (gui, kv, seals) = app(&e);
    tokio::spawn(async move {
        loop {
            if let Some(first) = approval::list_pending(&gui, &kv).await.unwrap().first() {
                sqlx::query("UPDATE mcp_pending_approvals SET arguments_json = '{}' WHERE id = ?")
                    .bind(&first.id)
                    .execute(&gui)
                    .await
                    .unwrap();
                approval::decide(&gui, &*seals, &first.id, ApprovalDecision::Approve, None)
                    .await
                    .unwrap();
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let err = gate::request_approval(&e.pool, &e.kv, sealer(&e), request(&e))
        .await
        .unwrap_err();
    assert!(matches!(err, RanchError::ApprovalDenied { .. }), "{err:?}");
}
