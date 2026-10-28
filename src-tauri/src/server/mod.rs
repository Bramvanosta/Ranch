//! The HTTP shell (one core, two shells).
//!
//! `ranch-server` serves the built frontend and one API route,
//! `POST /api/invoke/{command}`, which runs the same `*_inner` function as
//! the Tauri command of that name ([`dispatch`]). Everything but `/healthz`
//! sits behind HTTP Basic auth; the API also requires a custom header that a
//! browser cannot be made to send cross-site.
//!
//! Security model: no authentication, no start (unless
//! `RANCH_ALLOW_NO_AUTH=1`). Two ways to authenticate, both meant to sit
//! behind a TLS-terminating reverse proxy:
//!   - **HTTP Basic**: one shared account, checked in constant time;
//!   - **OpenID Connect** (`RANCH_OIDC_*`: Pocket ID, Authentik, Keycloak…):
//!     authorization code + PKCE, a verified ID token, then a signed,
//!     stateless session cookie (`HttpOnly`, `SameSite=Lax`, `Secure` on
//!     https). With OIDC on, the Basic account, if a password is set, stays
//!     as a break-glass and for scripts.
//! No MCP: agents need a human in front of a GUI to approve writes, and that
//! is the desktop app's job.

pub mod config;
pub mod dispatch;
pub mod oidc;
pub mod session;

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tower_http::services::{ServeDir, ServeFile};

use crate::adapters::secrets::FileSecretStore;
use crate::adapters::sqlite::Db;
use crate::app_state::AppState;
use crate::contract::errors::RanchErrorDto;

pub use config::{Auth, OidcConfig, ServerConfig};
use oidc::{OidcClient, OidcError};
use session::{LoginState, Session, SessionKey, LOGIN_COOKIE, SESSION_COOKIE};

/// What a browser cannot add to a cross-site request without a CORS
/// preflight, which this server never answers. Basic credentials are sent
/// automatically by the browser, so without this any page could POST here.
pub const CSRF_HEADER: &str = "x-ranch-request";

#[derive(Clone)]
struct Shell {
    app: Arc<AppState>,
    /// SHA-256 of `user:password`; `None` = no Basic account.
    credentials: Option<[u8; 32]>,
    /// `Some` = OpenID Connect sign-in.
    oidc: Option<Arc<OidcClient>>,
    session_key: Arc<SessionKey>,
    session_ttl_secs: u64,
    secrets_encrypted: bool,
}

impl Shell {
    fn auth_mode(&self) -> &'static str {
        match (&self.oidc, &self.credentials) {
            (Some(_), _) => "oidc",
            (None, Some(_)) => "basic",
            (None, None) => "none",
        }
    }

    /// The live session carried by these headers, if any.
    fn session(&self, headers: &HeaderMap) -> Option<Session> {
        let cookies = headers.get(header::COOKIE).and_then(|v| v.to_str().ok());
        let token = session::cookie(cookies, SESSION_COOKIE)?;
        self.session_key
            .open::<Session>("session", token)
            .filter(Session::is_live)
    }

    fn set_cookie(&self, name: &str, value: &str, path: &str, max_age: u64) -> HeaderValue {
        let secure = self
            .oidc
            .as_ref()
            .is_some_and(|o| o.config().secure_cookies);
        let cookie = format!(
            "{name}={value}; Path={path}; Max-Age={max_age}; HttpOnly; SameSite=Lax{}",
            if secure { "; Secure" } else { "" }
        );
        HeaderValue::from_str(&cookie).expect("cookie values are base64url")
    }
}

/// The length below which a secret key is too easy to guess offline.
const MIN_KEY_LEN: usize = 32;

pub async fn run(config: ServerConfig) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(&config.data_dir)
        .map_err(|e| format!("cannot create {}: {e}", config.data_dir.display()))?;
    let db = Db::init(&config.data_dir).await?;
    let store = FileSecretStore::new(&config.data_dir, config.secret_key.as_deref());

    if matches!(config.auth, Auth::None) {
        tracing::warn!(
            "RANCH_ALLOW_NO_AUTH=1: anyone who can reach {} can pause queues and delete jobs",
            config.bind
        );
    }
    if !store.is_encrypted() {
        tracing::warn!(
            "RANCH_SECRET_KEY is not set: Redis passwords are stored in clear in {}",
            config.data_dir.display()
        );
    }
    // Both keys are hashed, not stretched: a short one can be guessed offline
    // from a session cookie (every signed-in user has one) or a copy of /data.
    for (name, value) in [
        ("RANCH_SECRET_KEY", config.secret_key.as_deref()),
        ("RANCH_SESSION_SECRET", config.session_secret.as_deref()),
    ] {
        if value.is_some_and(|v| !v.is_empty() && v.len() < MIN_KEY_LEN) {
            tracing::warn!(
                "{name} is shorter than {MIN_KEY_LEN} characters: use a random value, e.g. `openssl rand -base64 32`"
            );
        }
    }
    if let Auth::Oidc { oidc, basic } = &config.auth {
        tracing::info!(
            issuer = %oidc.issuer,
            callback = %oidc.redirect_url,
            break_glass_password = basic.is_some(),
            "OIDC sign-in enabled; register the callback URL at the provider"
        );
        if oidc.allowed_emails.is_empty() && oidc.allowed_groups.is_empty() {
            tracing::info!(
                "no RANCH_OIDC_ALLOWED_EMAILS / _GROUPS: whoever the provider lets into this client may use Ranch"
            );
        }
        if config.session_secret.is_none() {
            tracing::warn!(
                "neither RANCH_SESSION_SECRET nor RANCH_SECRET_KEY is set: sessions end when the server restarts"
            );
        }
    }
    if !config.static_dir.join("index.html").is_file() {
        tracing::warn!(
            "no index.html in {}: the API works, the UI will not (set RANCH_STATIC_DIR)",
            config.static_dir.display()
        );
    }

    // No keychain here, so no key: nothing is sealed. The server runs no
    // agents, and the seals only matter to `ranch-mcp`.
    let sealer = Arc::new(crate::adapters::hmac_sealer::HmacSealer::unsealed());
    let app = Arc::new(AppState::new(db, Arc::new(store), sealer));
    let router = router(app.clone(), &config);

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!("ranch-server listening on http://{}", config.bind);
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // `docker stop`: close the Redis connections and SSH tunnels cleanly.
    app.connections.shutdown_all().await;
    Ok(())
}

pub fn router(app: Arc<AppState>, config: &ServerConfig) -> Router {
    let secrets_encrypted = config.secret_key.is_some();
    let digest = |user: &str, pass: &str| -> [u8; 32] {
        Sha256::digest(format!("{user}:{pass}").as_bytes()).into()
    };
    let (credentials, oidc) = match &config.auth {
        Auth::Basic { username, password } => (Some(digest(username, password)), None),
        Auth::Oidc { oidc, basic } => (
            basic.as_ref().map(|(u, p)| digest(u, p)),
            Some(Arc::new(OidcClient::new(oidc.clone()))),
        ),
        Auth::None => (None, None),
    };
    let (session_key, _) = SessionKey::from_secret(config.session_secret.as_deref());
    let shell = Shell {
        app,
        credentials,
        oidc,
        session_key: Arc::new(session_key),
        session_ttl_secs: config.session_ttl_hours * 3600,
        secrets_encrypted,
    };

    let api = Router::new()
        .route("/invoke/{command}", post(invoke))
        .route("/runtime", get(runtime))
        .layer(middleware::from_fn(require_csrf_header));

    let index = config.static_dir.join("index.html");
    let spa = ServeDir::new(&config.static_dir).fallback(ServeFile::new(index));

    Router::new()
        .nest("/api", api)
        .fallback_service(spa)
        .layer(middleware::from_fn_with_state(shell.clone(), require_auth))
        // Outside the auth layer: a container orchestrator has no password…
        .route("/healthz", get(|| async { "ok" }))
        // …and signing in cannot require being signed in.
        .route("/auth/login", get(auth_login))
        .route("/auth/callback", get(auth_callback))
        .route("/auth/logout", get(auth_logout).post(auth_logout))
        .layer(middleware::from_fn(security_headers))
        .with_state(shell)
}

async fn invoke(
    State(shell): State<Shell>,
    Path(command): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    let body = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    match dispatch::dispatch(&shell.app, &command, body).await {
        Ok(value) => Json(value).into_response(),
        Err(dto) => (status_for(&dto), Json(dto)).into_response(),
    }
}

/// What the frontend needs to know about where it runs, and who is there.
async fn runtime(State(shell): State<Shell>, headers: HeaderMap) -> Json<Value> {
    let user = shell
        .session(&headers)
        .map(|s| json!({ "name": s.name, "email": s.email }));
    Json(json!({
        "runtime": "server",
        "version": env!("CARGO_PKG_VERSION"),
        "auth": shell.auth_mode(),
        "user": user,
        "logoutUrl": shell.oidc.as_ref().map(|_| "/auth/logout"),
        "secretsEncrypted": shell.secrets_encrypted,
    }))
}

/// The body is what the UI reads (`kind`, `message`); the status is for
/// everything else that looks at HTTP: logs, proxies, `curl -f`.
fn status_for(dto: &RanchErrorDto) -> StatusCode {
    match dto.kind.as_str() {
        dispatch::KIND_UNKNOWN_COMMAND => StatusCode::NOT_FOUND,
        dispatch::KIND_DESKTOP_ONLY => StatusCode::NOT_IMPLEMENTED,
        k if k.ends_with("NotFound") => StatusCode::NOT_FOUND,
        "connectionLocked" => StatusCode::LOCKED,
        "invalidInput" | "invalidRedisUrl" | "invalidJobState" | "groupNotEmpty" => {
            StatusCode::BAD_REQUEST
        }
        "redis"
        | "sshAuthFailed"
        | "sshKeyFileError"
        | "sshTunnelFailed"
        | "schemaNotRecognized" => StatusCode::BAD_GATEWAY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn require_auth(State(shell): State<Shell>, request: Request, next: Next) -> Response {
    if shell.credentials.is_none() && shell.oidc.is_none() {
        return next.run(request).await; // RANCH_ALLOW_NO_AUTH=1
    }
    let given = request.headers().get(header::AUTHORIZATION);
    if let Some(expected) = &shell.credentials {
        if is_authorized(given, expected) {
            return next.run(request).await;
        }
    }
    if shell.oidc.is_some() && shell.session(request.headers()).is_some() {
        return next.run(request).await;
    }

    if given.is_some() {
        // Wrong credentials (not the browser's first, header-less request):
        // make guessing slow.
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    if shell.oidc.is_none() {
        return (
            StatusCode::UNAUTHORIZED,
            [(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Basic realm=\"Ranch\", charset=\"UTF-8\""),
            )],
            "Ranch: authentication required\n",
        )
            .into_response();
    }

    // OIDC: the API says where to sign in (no `WWW-Authenticate: Basic`, or
    // the browser would pop its own dialog); a page goes there by itself.
    let path = request.uri().path();
    if path.starts_with("/api/") {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "kind": "unauthorized",
                "message": "You are signed out of Ranch.",
                "docUrl": null,
                "loginUrl": "/auth/login",
            })),
        )
            .into_response();
    }
    let wanted = request
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let mut login = reqwest::Url::parse("http://ranch/auth/login").expect("a constant URL");
    login
        .query_pairs_mut()
        .append_pair("return_to", &session::safe_return_to(Some(wanted)));
    let location = format!("{}?{}", login.path(), login.query().unwrap_or_default());
    Redirect::to(&location).into_response()
}

#[derive(serde::Deserialize)]
struct LoginQuery {
    return_to: Option<String>,
}

/// Step 1: off to the provider, remembering (in a signed, short-lived
/// cookie) what the callback will have to check.
async fn auth_login(State(shell): State<Shell>, Query(query): Query<LoginQuery>) -> Response {
    let Some(oidc) = &shell.oidc else {
        return Redirect::to("/").into_response();
    };
    let login = LoginState {
        state: session::random_token(),
        nonce: session::random_token(),
        verifier: session::random_token(),
        return_to: session::safe_return_to(query.return_to.as_deref()),
        exp: session::now() + 600,
    };
    let url = match oidc
        .authorization_url(
            &login.state,
            &login.nonce,
            &session::pkce_challenge(&login.verifier),
        )
        .await
    {
        Ok(url) => url,
        Err(e) => return auth_failure(StatusCode::BAD_GATEWAY, &e.to_string()),
    };
    let cookie = shell.set_cookie(
        LOGIN_COOKIE,
        &shell.session_key.seal("login", &login),
        "/auth",
        600,
    );
    ([(header::SET_COOKIE, cookie)], Redirect::to(&url)).into_response()
}

#[derive(serde::Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Step 2: back from the provider: check `state`, trade the code for a
/// verified identity, open the session.
async fn auth_callback(
    State(shell): State<Shell>,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let Some(oidc) = &shell.oidc else {
        return Redirect::to("/").into_response();
    };
    if let Some(error) = query.error {
        let detail = query.error_description.unwrap_or_default();
        return auth_failure(
            StatusCode::FORBIDDEN,
            &format!("The identity provider refused the sign-in: {error} {detail}"),
        );
    }
    let cookies = headers.get(header::COOKIE).and_then(|v| v.to_str().ok());
    let login = session::cookie(cookies, LOGIN_COOKIE)
        .and_then(|token| shell.session_key.open::<LoginState>("login", token))
        .filter(|login| login.exp > session::now());
    let (Some(login), Some(code), Some(state)) = (login, query.code, query.state) else {
        return auth_failure(
            StatusCode::BAD_REQUEST,
            "This sign-in link is no longer valid (it expired, or it was opened in another browser).",
        );
    };
    // The `state` we sent must come back: otherwise someone else started this.
    if !bool::from(state.as_bytes().ct_eq(login.state.as_bytes())) {
        return auth_failure(
            StatusCode::BAD_REQUEST,
            "This sign-in does not match the one that was started here.",
        );
    }

    let identity = match oidc.exchange(&code, &login.verifier, &login.nonce).await {
        Ok(identity) => identity,
        Err(e) => {
            let status = match e {
                OidcError::Provider(_) => StatusCode::BAD_GATEWAY,
                OidcError::Rejected(_) | OidcError::NotAllowed(_) => StatusCode::FORBIDDEN,
            };
            tracing::warn!(error = %e, "OIDC sign-in refused");
            return auth_failure(status, &e.to_string());
        }
    };
    tracing::info!(sub = %identity.sub, email = ?identity.email, "signed in");

    let session = Session {
        sub: identity.sub,
        name: identity.name,
        email: identity.email,
        exp: session::now() + shell.session_ttl_secs,
    };
    let mut response = Redirect::to(&login.return_to).into_response();
    let headers = response.headers_mut();
    headers.append(
        header::SET_COOKIE,
        shell.set_cookie(
            SESSION_COOKIE,
            &shell.session_key.seal("session", &session),
            "/",
            shell.session_ttl_secs,
        ),
    );
    headers.append(
        header::SET_COOKIE,
        shell.set_cookie(LOGIN_COOKIE, "", "/auth", 0),
    );
    response
}

async fn auth_logout(State(shell): State<Shell>) -> Response {
    let page = Html(
        "<!doctype html><meta charset=utf-8><title>Ranch</title>\
         <body style=\"font:16px system-ui;display:grid;place-items:center;height:100vh;margin:0\">\
         <p>You are signed out of Ranch. <a href=\"/\">Sign in again</a></p>",
    );
    (
        [(
            header::SET_COOKIE,
            shell.set_cookie(SESSION_COOKIE, "", "/", 0),
        )],
        page,
    )
        .into_response()
}

/// A sign-in that did not work, said plainly, with a way to try again.
fn auth_failure(status: StatusCode, message: &str) -> Response {
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let page = format!(
        "<!doctype html><meta charset=utf-8><title>Ranch sign-in</title>\
         <body style=\"font:16px system-ui;max-width:36rem;margin:20vh auto;padding:0 1rem\">\
         <h1 style=\"font-size:1.25rem\">Ranch could not sign you in</h1>\
         <p>{escaped}</p><p><a href=\"/auth/login\">Try again</a></p>"
    );
    (status, Html(page)).into_response()
}

/// Both sides are hashed first: the comparison is over equal lengths, and
/// its timing says nothing about how much of the password was right.
fn is_authorized(header: Option<&HeaderValue>, expected: &[u8; 32]) -> bool {
    let Some(encoded) = header
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Basic "))
    else {
        return false;
    };
    let Ok(decoded) = B64.decode(encoded.trim()) else {
        return false;
    };
    let given: [u8; 32] = Sha256::digest(&decoded).into();
    given.ct_eq(expected).into()
}

async fn require_csrf_header(request: Request, next: Next) -> Response {
    if request.headers().contains_key(CSRF_HEADER) {
        return next.run(request).await;
    }
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "kind": "forbidden",
            "message": format!("API requests must carry the `{CSRF_HEADER}` header"),
            "docUrl": null,
        })),
    )
        .into_response()
}

const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; \
     style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; \
     connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; \
     frame-ancestors 'none'";

async fn security_headers(request: Request, next: Next) -> Response {
    let is_api = request.uri().path().starts_with("/api/");
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    // The same policy as the desktop webview: scripts from this origin only,
    // nothing loaded from elsewhere, never framed. Styles stay inline-able
    // (the toasts and the charts insert theirs at run time).
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    if is_api {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutting down");
}

#[cfg(test)]
mod test_keys;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_oidc;
