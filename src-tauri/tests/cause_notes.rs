//! Resolution notes: an agent annotates a cause through
//! the MCP tool; Ranch lists and dismisses notes; a connection read-only for
//! agents refuses them; nothing needs an approval.

use std::sync::Arc;

use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::mcp_safety::set_connection_mcp_readonly_inner;
use ranch_lib::core::cause_notes::{self, ListCauseNotesArgs};
use ranch_lib::core::connection_actor::ConnectionRegistry;
use ranch_lib::core::db::Db;
use ranch_lib::core::keychain::{InMemoryKeychain, KeychainAdapter};
use ranch_lib::core::mcp::server::{AnnotateCauseInput, McpQueueRef};
use ranch_lib::core::mcp::{McpServerState, RanchMcpServer};
use rmcp::handler::server::wrapper::Parameters;
use serde_json::Value;

struct Env {
    _dir: tempfile::TempDir,
    db: Db,
    server: RanchMcpServer,
    connection_id: String,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let keychain: Arc<dyn KeychainAdapter> = Arc::new(InMemoryKeychain::new());
    let connection = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "staging".into(),
            host: "127.0.0.1".into(),
            port: 1,
            db: 0,
            username: None,
            password: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
            ssh_password: None,
            group: GroupSelector::named("Lumen"),
            environment: EnvironmentSelector::named("Staging", "env-staging"),
        },
        db.pool(),
        &keychain,
    )
    .await
    .unwrap();
    let server = RanchMcpServer::new(McpServerState {
        pool: db.pool().clone(),
        keychain,
        registry: Arc::new(ConnectionRegistry::new()),
    });
    Env {
        _dir: dir,
        db,
        server,
        connection_id: connection.id,
    }
}

fn note(env: &Env, text: &str) -> Parameters<AnnotateCauseInput> {
    Parameters(AnnotateCauseInput {
        connection_id: env.connection_id.clone(),
        queue_ref: McpQueueRef {
            prefix: "bull".into(),
            name: "emails".into(),
        },
        signature: "connect ECONNREFUSED <addr>".into(),
        text: text.into(),
        steps: vec![
            "Restart the SMTP relay".into(),
            "  ".into(),
            "Retry the jobs".into(),
        ],
        jobs_read: vec!["41".into(), "42".into()],
    })
}

#[tokio::test]
async fn an_agent_annotates_a_cause_without_approval_and_ranch_lists_it() {
    let env = env().await;
    env.server
        .set_client_info_for_test("claude-ai", "0.14")
        .await;
    let answer = env
        .server
        .annotate_cause(note(&env, "  The SMTP relay is down since the deploy.  "))
        .await
        .expect("no approval needed");
    let stored: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(stored["text"], "The SMTP relay is down since the deploy.");
    assert_eq!(stored["clientHint"], "claude-ai");

    let notes = cause_notes::list(
        env.db.pool(),
        ListCauseNotesArgs {
            connection_id: Some(env.connection_id.clone()),
            include_dismissed: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(notes.len(), 1);
    let n = &notes[0];
    assert_eq!(
        n.steps,
        ["Restart the SMTP relay", "Retry the jobs"],
        "blank steps dropped"
    );
    assert_eq!(n.jobs_read, ["41", "42"]);
    assert_eq!(n.queue_ref.name, "emails");
    assert_eq!(n.connection_name.as_deref(), Some("staging"));
    assert!(n.dismissed_at.is_none());

    cause_notes::dismiss(env.db.pool(), &n.id).await.unwrap();
    let shown = cause_notes::list(env.db.pool(), ListCauseNotesArgs::default())
        .await
        .unwrap();
    assert!(shown.is_empty(), "dismissed notes are hidden by default");
    let all = cause_notes::list(
        env.db.pool(),
        ListCauseNotesArgs {
            connection_id: None,
            include_dismissed: true,
        },
    )
    .await
    .unwrap();
    assert!(all[0].dismissed_at.is_some());
}

#[tokio::test]
async fn a_connection_read_only_for_agents_refuses_notes() {
    let env = env().await;
    set_connection_mcp_readonly_inner(&env.connection_id, Some(true), env.db.pool())
        .await
        .unwrap();
    let err = env
        .server
        .annotate_cause(note(&env, "anything"))
        .await
        .unwrap_err();
    assert_eq!(
        err.data
            .as_ref()
            .and_then(|d| d.get("kind"))
            .and_then(Value::as_str),
        Some("blockedReadonly")
    );
    assert!(
        cause_notes::list(env.db.pool(), ListCauseNotesArgs::default())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_note_needs_text_and_a_known_connection() {
    let env = env().await;
    let err = env
        .server
        .annotate_cause(note(&env, "   "))
        .await
        .unwrap_err();
    assert!(err.message.contains("needs some text"), "{}", err.message);
    let long = "x".repeat(cause_notes::MAX_TEXT_CHARS + 1);
    assert!(env.server.annotate_cause(note(&env, &long)).await.is_err());
    let mut unknown = note(&env, "text");
    unknown.0.connection_id = "nope".into();
    assert!(env.server.annotate_cause(unknown).await.is_err());
}
