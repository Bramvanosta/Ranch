//! The OIDC sign-in, end to end, against a mock identity provider: discovery,
//! JWKS, a token endpoint that checks the PKCE verifier and the client secret
//! and hands out whatever ID token the test tells it to, so the refusals
//! can be tested as well as the welcome.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Form, State};
use axum::http::{header, Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use tower::ServiceExt;

use super::session::{pkce_challenge, SESSION_COOKIE};
use super::test_keys::{TEST_RSA_E, TEST_RSA_N, TEST_RSA_PEM};
use super::*;
use crate::adapters::hmac_sealer::HmacSealer;
use crate::adapters::secrets::InMemoryKeychain;

const CLIENT_ID: &str = "ranch";
const CLIENT_SECRET: &str = "shh";

/// What the mock provider will put in the next ID token.
#[derive(Clone)]
struct Next {
    claims: Value,
    alg: Algorithm,
    kid: &'static str,
}

#[derive(Clone)]
struct Idp {
    issuer: Arc<Mutex<String>>,
    next: Arc<Mutex<Option<Next>>>,
    /// The PKCE challenge the test saw in the authorization URL.
    challenge: Arc<Mutex<Option<String>>>,
}

async fn start_idp() -> (Idp, String) {
    let idp = Idp {
        issuer: Arc::new(Mutex::new(String::new())),
        next: Arc::new(Mutex::new(None)),
        challenge: Arc::new(Mutex::new(None)),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    *idp.issuer.lock().unwrap() = issuer.clone();

    let app = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(|State(idp): State<Idp>| async move {
                let issuer = idp.issuer.lock().unwrap().clone();
                Json(json!({
                    "issuer": issuer,
                    "authorization_endpoint": format!("{issuer}/authorize"),
                    "token_endpoint": format!("{issuer}/token"),
                    "jwks_uri": format!("{issuer}/jwks"),
                    "token_endpoint_auth_methods_supported": ["client_secret_basic"],
                }))
            }),
        )
        .route(
            "/jwks",
            get(|| async {
                Json(json!({ "keys": [
                    // A key this build cannot use must not break the others.
                    { "kty": "XYZ", "kid": "weird" },
                    { "kty": "RSA", "kid": "k1", "use": "sig", "alg": "RS256", "n": TEST_RSA_N, "e": TEST_RSA_E },
                ]}))
            }),
        )
        .route("/token", post(token))
        .with_state(idp.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (idp, issuer)
}

async fn token(
    State(idp): State<Idp>,
    headers: axum::http::HeaderMap,
    Form(form): Form<std::collections::HashMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    let expected = format!(
        "Basic {}",
        STANDARD.encode(format!("{CLIENT_ID}:{CLIENT_SECRET}"))
    );
    let authed = headers
        .get(header::AUTHORIZATION)
        .is_some_and(|h| h.to_str().unwrap() == expected);
    let pkce_ok = idp.challenge.lock().unwrap().as_deref()
        == form
            .get("code_verifier")
            .map(|v| pkce_challenge(v))
            .as_deref();
    if !authed
        || !pkce_ok
        || form.get("grant_type").map(String::as_str) != Some("authorization_code")
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid_grant" })),
        );
    }
    let next = idp
        .next
        .lock()
        .unwrap()
        .clone()
        .expect("the test sets the next token");
    let mut header = Header::new(next.alg);
    header.kid = Some(next.kid.to_string());
    let key = match next.alg {
        Algorithm::HS256 => EncodingKey::from_secret(TEST_RSA_N.as_bytes()),
        _ => EncodingKey::from_rsa_pem(TEST_RSA_PEM.as_bytes()).unwrap(),
    };
    let id_token = encode(&header, &next.claims, &key).unwrap();
    (
        StatusCode::OK,
        Json(json!({ "access_token": "at", "token_type": "Bearer", "id_token": id_token })),
    )
}

struct Harness {
    app: Router,
    idp: Idp,
    issuer: String,
}

async fn harness(
    allowed_emails: &[&str],
    basic: Option<(&str, &str)>,
) -> (Harness, tempfile::TempDir) {
    let (idp, issuer) = start_idp().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("dist")).unwrap();
    std::fs::write(dir.path().join("dist/index.html"), "<html>ranch</html>").unwrap();
    let state = Arc::new(AppState::new(
        Db::init(dir.path()).await.unwrap(),
        Arc::new(InMemoryKeychain::new()),
        Arc::new(HmacSealer::unsealed()),
    ));
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        data_dir: dir.path().to_path_buf(),
        static_dir: dir.path().join("dist"),
        auth: Auth::Oidc {
            oidc: OidcConfig {
                issuer: issuer.clone(),
                client_id: CLIENT_ID.into(),
                client_secret: CLIENT_SECRET.into(),
                redirect_url: "https://ranch.example.com/auth/callback".into(),
                scopes: "openid profile email".into(),
                allowed_emails: allowed_emails.iter().map(|e| e.to_string()).collect(),
                allowed_groups: vec!["admins".into()]
                    .into_iter()
                    .filter(|_| !allowed_emails.is_empty())
                    .collect(),
                secure_cookies: true,
            },
            basic: basic.map(|(u, p)| (u.to_string(), p.to_string())),
        },
        secret_key: None,
        session_secret: Some("test-session-secret".into()),
        session_ttl_hours: 12,
    };
    let app = router(state, &config);
    (Harness { app, idp, issuer }, dir)
}

fn cookie_from(response: &Response, name: &str) -> Option<String> {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with(&format!("{name}=")))
        .map(|v| v.split(';').next().unwrap().to_string())
}

/// `/auth/login` → (login cookie, state, nonce); the challenge is handed to the mock.
async fn begin(h: &Harness, return_to: &str) -> (String, String, String) {
    let response = h
        .app
        .clone()
        .oneshot(
            Request::get(format!("/auth/login?return_to={return_to}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        location.starts_with(&format!("{}/authorize?", h.issuer)),
        "{location}"
    );
    let url = reqwest::Url::parse(&location).unwrap();
    let param = |k: &str| {
        url.query_pairs()
            .find(|(n, _)| n == k)
            .unwrap()
            .1
            .to_string()
    };
    assert_eq!(param("response_type"), "code");
    assert_eq!(param("client_id"), CLIENT_ID);
    assert_eq!(
        param("redirect_uri"),
        "https://ranch.example.com/auth/callback"
    );
    assert_eq!(param("code_challenge_method"), "S256");
    *h.idp.challenge.lock().unwrap() = Some(param("code_challenge"));
    let cookie = cookie_from(&response, "ranch_login").expect("a login-state cookie");
    (cookie, param("state"), param("nonce"))
}

fn claims(h: &Harness, nonce: &str, email: &str) -> Value {
    json!({
        "iss": h.issuer, "aud": CLIENT_ID, "sub": "user-1", "nonce": nonce,
        "name": "Alex", "email": email, "email_verified": true, "groups": ["family"],
        "exp": session::now() + 300, "iat": session::now(),
    })
}

async fn callback(h: &Harness, login_cookie: &str, state: &str) -> Response {
    h.app
        .clone()
        .oneshot(
            Request::get(format!("/auth/callback?code=abc&state={state}"))
                .header(header::COOKIE, login_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

fn api_call(cookie: Option<&str>) -> Request<Body> {
    let mut request = Request::post("/api/invoke/list_groups")
        .header(header::CONTENT_TYPE, "application/json")
        .header(CSRF_HEADER, "1");
    if let Some(c) = cookie {
        request = request.header(header::COOKIE, c);
    }
    request.body(Body::from("{}")).unwrap()
}

#[tokio::test]
async fn signed_out_a_page_goes_to_the_provider_and_the_api_says_where_to_sign_in() {
    let (h, _dir) = harness(&[], None).await;
    let page = h
        .app
        .clone()
        .oneshot(
            Request::get("/connections/c1?view=queues")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        page.headers()[header::LOCATION],
        "/auth/login?return_to=%2Fconnections%2Fc1%3Fview%3Dqueues"
    );

    let api = h.app.clone().oneshot(api_call(None)).await.unwrap();
    assert_eq!(api.status(), StatusCode::UNAUTHORIZED);
    // No Basic challenge: the browser must not pop its own dialog over the app.
    assert!(api.headers().get(header::WWW_AUTHENTICATE).is_none());
    let bytes = axum::body::to_bytes(api.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["kind"], "unauthorized");
    assert_eq!(body["loginUrl"], "/auth/login");
}

#[tokio::test]
async fn the_whole_sign_in_then_the_session_opens_the_api_and_sign_out_closes_it() {
    let (h, _dir) = harness(&[], None).await;
    let (login_cookie, state, nonce) = begin(&h, "/connections/c1").await;
    *h.idp.next.lock().unwrap() = Some(Next {
        claims: claims(&h, &nonce, "alex@example.com"),
        alg: Algorithm::RS256,
        kid: "k1",
    });

    let response = callback(&h, &login_cookie, &state).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], "/connections/c1");
    let set = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .find(|v| v.starts_with(SESSION_COOKIE))
        .unwrap();
    assert!(set.contains("HttpOnly") && set.contains("SameSite=Lax") && set.contains("Secure"));
    let session_cookie = cookie_from(&response, SESSION_COOKIE).unwrap();

    let api = h
        .app
        .clone()
        .oneshot(api_call(Some(&session_cookie)))
        .await
        .unwrap();
    assert_eq!(api.status(), StatusCode::OK);

    let runtime = h
        .app
        .clone()
        .oneshot(
            Request::get("/api/runtime")
                .header(CSRF_HEADER, "1")
                .header(header::COOKIE, &session_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(runtime.into_body(), usize::MAX)
        .await
        .unwrap();
    let info: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(info["auth"], "oidc");
    assert_eq!(info["user"]["email"], "alex@example.com");
    assert_eq!(info["logoutUrl"], "/auth/logout");

    // A forged cookie is no session.
    let forged = format!("{}x", session_cookie);
    let refused = h
        .app
        .clone()
        .oneshot(api_call(Some(&forged)))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);

    let logout = h
        .app
        .clone()
        .oneshot(Request::get("/auth/logout").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(logout.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
}

#[tokio::test]
async fn what_does_not_check_out_is_refused() {
    let (h, _dir) = harness(&[], None).await;

    // A `state` that is not the one this browser was given.
    let (login_cookie, _state, nonce) = begin(&h, "/").await;
    *h.idp.next.lock().unwrap() = Some(Next {
        claims: claims(&h, &nonce, "alex@example.com"),
        alg: Algorithm::RS256,
        kid: "k1",
    });
    assert_eq!(
        callback(&h, &login_cookie, "someone-elses").await.status(),
        StatusCode::BAD_REQUEST
    );
    // No login cookie at all (the link opened in another browser).
    assert_eq!(
        callback(&h, "", "x").await.status(),
        StatusCode::BAD_REQUEST
    );

    // An ID token that answers another sign-in.
    let (login_cookie, state, _nonce) = begin(&h, "/").await;
    *h.idp.next.lock().unwrap() = Some(Next {
        claims: claims(&h, "another-nonce", "alex@example.com"),
        alg: Algorithm::RS256,
        kid: "k1",
    });
    let response = callback(&h, &login_cookie, &state).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(cookie_from(&response, SESSION_COOKIE).is_none());

    // Meant for another client; from another issuer; expired.
    for (field, value) in [
        ("aud", json!("someone-else")),
        ("iss", json!("https://evil.example")),
        ("exp", json!(session::now() - 3600)),
    ] {
        let (login_cookie, state, nonce) = begin(&h, "/").await;
        let mut c = claims(&h, &nonce, "alex@example.com");
        c[field] = value;
        *h.idp.next.lock().unwrap() = Some(Next {
            claims: c,
            alg: Algorithm::RS256,
            kid: "k1",
        });
        assert_eq!(
            callback(&h, &login_cookie, &state).await.status(),
            StatusCode::FORBIDDEN,
            "{field}"
        );
    }

    // "Signed" with HS256 and the public key: the algorithm-confusion attack.
    let (login_cookie, state, nonce) = begin(&h, "/").await;
    *h.idp.next.lock().unwrap() = Some(Next {
        claims: claims(&h, &nonce, "alex@example.com"),
        alg: Algorithm::HS256,
        kid: "k1",
    });
    assert_eq!(
        callback(&h, &login_cookie, &state).await.status(),
        StatusCode::FORBIDDEN
    );

    // Signed with a key the provider does not publish.
    let (login_cookie, state, nonce) = begin(&h, "/").await;
    *h.idp.next.lock().unwrap() = Some(Next {
        claims: claims(&h, &nonce, "alex@example.com"),
        alg: Algorithm::RS256,
        kid: "unknown",
    });
    assert_eq!(
        callback(&h, &login_cookie, &state).await.status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn allow_lists_decide_who_gets_in() {
    let (h, _dir) = harness(&["alex@example.com"], None).await;
    for (email, groups, expected) in [
        ("alex@example.com", json!([]), StatusCode::SEE_OTHER),
        ("ALEX@example.com", json!([]), StatusCode::SEE_OTHER),
        ("stranger@example.com", json!([]), StatusCode::FORBIDDEN),
        // Not on the e-mail list, but in an allowed group: either is enough.
        (
            "colleague@example.com",
            json!(["admins"]),
            StatusCode::SEE_OTHER,
        ),
    ] {
        let (login_cookie, state, nonce) = begin(&h, "/").await;
        let mut c = claims(&h, &nonce, email);
        c["groups"] = groups;
        *h.idp.next.lock().unwrap() = Some(Next {
            claims: c,
            alg: Algorithm::RS256,
            kid: "k1",
        });
        assert_eq!(
            callback(&h, &login_cookie, &state).await.status(),
            expected,
            "{email}"
        );
    }

    // An allowed address the provider did not verify proves nothing; the
    // string "true" some providers send counts as verified.
    for (verified, expected) in [
        (json!(false), StatusCode::FORBIDDEN),
        (Value::Null, StatusCode::FORBIDDEN),
        (json!("true"), StatusCode::SEE_OTHER),
    ] {
        let (login_cookie, state, nonce) = begin(&h, "/").await;
        let mut c = claims(&h, &nonce, "alex@example.com");
        c["groups"] = json!([]);
        c["email_verified"] = verified.clone();
        *h.idp.next.lock().unwrap() = Some(Next {
            claims: c,
            alg: Algorithm::RS256,
            kid: "k1",
        });
        assert_eq!(
            callback(&h, &login_cookie, &state).await.status(),
            expected,
            "email_verified = {verified}"
        );
    }
}

#[tokio::test]
async fn the_break_glass_password_still_opens_the_api_when_one_is_set() {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    let basic = |p: &str| format!("Basic {}", STANDARD.encode(format!("ranch:{p}")));

    let (h, _dir) = harness(&[], Some(("ranch", "break-glass"))).await;
    let mut request = api_call(None);
    request
        .headers_mut()
        .insert(header::AUTHORIZATION, basic("break-glass").parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::OK
    );
    let mut request = api_call(None);
    request
        .headers_mut()
        .insert(header::AUTHORIZATION, basic("nope").parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );

    // Without a password configured, Basic is simply not a way in.
    let (h, _dir) = harness(&[], None).await;
    let mut request = api_call(None);
    request
        .headers_mut()
        .insert(header::AUTHORIZATION, basic("break-glass").parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_return_to_cannot_send_the_user_elsewhere() {
    let (h, _dir) = harness(&[], None).await;
    let (login_cookie, state, nonce) = begin(&h, "https%3A%2F%2Fevil.example").await;
    *h.idp.next.lock().unwrap() = Some(Next {
        claims: claims(&h, &nonce, "alex@example.com"),
        alg: Algorithm::RS256,
        kid: "k1",
    });
    let response = callback(&h, &login_cookie, &state).await;
    assert_eq!(response.headers()[header::LOCATION], "/");
}
