use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use crate::commands::registry;
use crate::core::keychain::InMemoryKeychain;

async fn app_state(dir: &std::path::Path) -> Arc<AppState> {
    Arc::new(AppState {
        db: Arc::new(Db::init(dir).await.unwrap()),
        keychain: Arc::new(InMemoryKeychain::new()),
        connections: Arc::new(ConnectionRegistry::new()),
    })
}

fn config(dir: &std::path::Path, auth: Auth) -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        data_dir: dir.to_path_buf(),
        static_dir: dir.join("dist"),
        auth,
        secret_key: None,
        session_secret: Some("test-session-secret".into()),
        session_ttl_hours: 12,
    }
}

fn basic(user: &str, password: &str) -> String {
    format!("Basic {}", B64.encode(format!("{user}:{password}")))
}

fn password_auth() -> Auth {
    Auth::Basic {
        username: "ranch".into(),
        password: "s3cret".into(),
    }
}

async fn body_json(response: Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn invoke_request(command: &str, body: Value, authorization: Option<&str>) -> Request<Body> {
    let mut request = Request::post(format!("/api/invoke/{command}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(CSRF_HEADER, "1");
    if let Some(a) = authorization {
        request = request.header(header::AUTHORIZATION, a);
    }
    request.body(Body::from(body.to_string())).unwrap()
}

/// The point of the whole shell: a command added to the app cannot be
/// forgotten here. Called with an empty body, a served command complains
/// about its input (or succeeds); only a missing arm says `unknownCommand`.
#[tokio::test]
async fn every_ui_command_is_served_or_desktop_only() {
    let dir = tempfile::tempdir().unwrap();
    let state = app_state(dir.path()).await;
    let mut unknown = Vec::new();
    for name in registry::ui_command_names() {
        if let Err(dto) = dispatch::dispatch(&state, name, json!({})).await {
            if dto.kind == dispatch::KIND_UNKNOWN_COMMAND {
                unknown.push(name);
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "commands neither served by the HTTP shell nor listed in DESKTOP_ONLY: {unknown:?}"
    );

    // …and the desktop-only list names real commands, with a reason.
    let all = registry::ui_command_names();
    for (name, why) in dispatch::DESKTOP_ONLY {
        assert!(all.contains(name), "{name} is not a command any more");
        assert!(!why.is_empty());
    }
}

#[tokio::test]
async fn everything_but_healthz_needs_the_password() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(
        app_state(dir.path()).await,
        &config(dir.path(), password_auth()),
    );

    let health = app
        .clone()
        .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);

    for authorization in [
        None,
        Some(basic("ranch", "wrong")),
        Some("Bearer x".to_string()),
    ] {
        let response = app
            .clone()
            .oneshot(invoke_request(
                "list_groups",
                json!({}),
                authorization.as_deref(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("Basic"));
    }

    // The UI itself is behind the password too.
    let page = app
        .clone()
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::UNAUTHORIZED);

    let ok = app
        .oneshot(invoke_request(
            "list_groups",
            json!({}),
            Some(&basic("ranch", "s3cret")),
        ))
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
}

#[tokio::test]
async fn the_api_refuses_a_request_a_foreign_page_could_forge() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(app_state(dir.path()).await, &config(dir.path(), Auth::None));
    // Credentials are attached by the browser; this header cannot be.
    let forged = Request::post("/api/invoke/list_groups")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let response = app.oneshot(forged).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_command_runs_the_real_thing_and_errors_keep_their_kind() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(app_state(dir.path()).await, &config(dir.path(), Auth::None));

    let created = app
        .clone()
        .oneshot(invoke_request(
            "create_group",
            json!({ "args": { "name": "Acme" } }),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    assert_eq!(body_json(created).await["name"], "Acme");

    let listed = app
        .clone()
        .oneshot(invoke_request("list_groups", json!({}), None))
        .await
        .unwrap();
    let groups = body_json(listed).await;
    assert!(groups
        .as_array()
        .unwrap()
        .iter()
        .any(|g| g["name"] == "Acme"));

    // Named-field commands (not `{ args }`) work the same.
    let set = app
        .clone()
        .oneshot(invoke_request(
            "set_kv_setting",
            json!({ "key": "theme", "value": "dark" }),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(set.status(), StatusCode::OK);
    let got = app
        .clone()
        .oneshot(invoke_request(
            "get_kv_setting",
            json!({ "key": "theme" }),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(body_json(got).await, json!("dark"));

    // A domain error: typed body, meaningful status.
    let missing = app
        .clone()
        .oneshot(invoke_request(
            "list_queues",
            json!({ "args": { "connectionId": "nope" } }),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(missing).await["kind"], "connectionNotFound");

    let bad_input = app
        .clone()
        .oneshot(invoke_request("create_group", json!({}), None))
        .await
        .unwrap();
    assert_eq!(bad_input.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(bad_input).await["kind"], "invalidInput");

    let desktop_only = app
        .clone()
        .oneshot(invoke_request("get_mcp_installer_status", json!({}), None))
        .await
        .unwrap();
    assert_eq!(desktop_only.status(), StatusCode::NOT_IMPLEMENTED);

    let unknown = app
        .oneshot(invoke_request("rm_rf", json!({}), None))
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn serves_the_frontend_with_an_spa_fallback_and_says_where_it_runs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("dist/assets")).unwrap();
    std::fs::write(dir.path().join("dist/index.html"), "<html>ranch</html>").unwrap();
    std::fs::write(dir.path().join("dist/assets/app.js"), "console.log(1)").unwrap();
    let app = router(app_state(dir.path()).await, &config(dir.path(), Auth::None));

    for path in ["/", "/assets/app.js", "/connections/c1/queues/emails"] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(response.headers()["x-frame-options"], "DENY");
    }

    let runtime = app
        .oneshot(
            Request::get("/api/runtime")
                .header(CSRF_HEADER, "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let info = body_json(runtime).await;
    assert_eq!(info["runtime"], "server");
    assert_eq!(info["auth"], "none");
}
