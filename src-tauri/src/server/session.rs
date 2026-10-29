//! Signed, stateless cookies: the OIDC session, and the short-lived state of
//! a login in progress.
//!
//! A token is `base64url(json) "." base64url(HMAC-SHA256(key, purpose ‖ "." ‖
//! json))`. The cookie is the session, the MAC makes it unforgeable, `exp`
//! makes it finite. The *purpose* is part of what is signed, so a login-state
//! cookie can never be replayed as a session. The server keeps only the ids
//! of sessions signed out before they expired: such a cookie, copied
//! somewhere, opens nothing.
//!
//! The key comes from `RANCH_SESSION_SECRET` (or, failing that,
//! `RANCH_SECRET_KEY`); without either it is random and sessions end when the
//! container restarts, and the server says so at start-up.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine;
use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::aead::OsRng;
use hmac::{Hmac, Mac};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SESSION_COOKIE: &str = "ranch_session";
pub const LOGIN_COOKIE: &str = "ranch_login";

pub struct SessionKey([u8; 32]);

impl SessionKey {
    /// `(key, persistent)`: not persistent means "random, for this process".
    pub fn from_secret(secret: Option<&str>) -> (Self, bool) {
        match secret.filter(|s| !s.is_empty()) {
            Some(secret) => {
                let digest = Sha256::digest(format!("ranch-session:{secret}").as_bytes());
                (Self(digest.into()), true)
            }
            None => {
                let mut key = [0u8; 32];
                OsRng.fill_bytes(&mut key);
                (Self(key), false)
            }
        }
    }

    fn mac(&self, purpose: &str, payload: &[u8]) -> Hmac<Sha256> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).expect("HMAC takes any key length");
        mac.update(purpose.as_bytes());
        mac.update(b".");
        mac.update(payload);
        mac
    }

    pub fn seal<T: Serialize>(&self, purpose: &str, value: &T) -> String {
        let payload = serde_json::to_vec(value).expect("session payloads are plain structs");
        let tag = self.mac(purpose, &payload).finalize().into_bytes();
        format!("{}.{}", B64URL.encode(&payload), B64URL.encode(tag))
    }

    /// `None` for anything that is not a token this key signed for this purpose.
    /// (Expiry is the caller's: see [`Session::is_live`].)
    pub fn open<T: DeserializeOwned>(&self, purpose: &str, token: &str) -> Option<T> {
        let (payload, tag) = token.split_once('.')?;
        let payload = B64URL.decode(payload).ok()?;
        let tag = B64URL.decode(tag).ok()?;
        // `verify_slice` compares in constant time.
        self.mac(purpose, &payload).verify_slice(&tag).ok()?;
        serde_json::from_slice(&payload).ok()
    }
}

/// Who is signed in. Kept small: it travels in every request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Session {
    /// Random, per sign-in: what signing out revokes. A cookie from a
    /// release without it has none, and is no session.
    #[serde(default)]
    pub id: String,
    pub sub: String,
    pub name: Option<String>,
    pub email: Option<String>,
    /// Unix seconds.
    pub exp: u64,
}

impl Session {
    pub fn is_live(&self) -> bool {
        !self.id.is_empty() && self.exp > now()
    }
}

/// What a login in progress must remember between `/auth/login` and
/// `/auth/callback`: the CSRF `state`, the ID-token `nonce`, the PKCE
/// verifier, and where the user was going.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginState {
    pub state: String,
    pub nonce: String,
    pub verifier: String,
    pub return_to: String,
    pub exp: u64,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 32 random bytes, URL-safe: a `state`, a `nonce`, a PKCE verifier.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    B64URL.encode(bytes)
}

/// PKCE `S256`: `base64url(sha256(verifier))`.
pub fn pkce_challenge(verifier: &str) -> String {
    B64URL.encode(Sha256::digest(verifier.as_bytes()))
}

/// The value of cookie `name` in a `Cookie:` header.
pub fn cookie<'a>(header: Option<&'a str>, name: &str) -> Option<&'a str> {
    header?.split(';').find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k == name).then_some(v)
    })
}

/// Only a path on this site: `/x`, never `//host`, `/\host` or a scheme: a
/// `return_to` is attacker-controlled and ends up in a `Location:`. No
/// backslash and no control character anywhere: browsers drop tabs and
/// newlines from URLs, so `/<TAB>/evil.example` would land on `//evil.example`.
pub fn safe_return_to(candidate: Option<&str>) -> String {
    match candidate {
        Some(path)
            if path.starts_with('/')
                && !path.starts_with("//")
                && !path.contains('\\')
                && !path.chars().any(char::is_control)
                && !path.starts_with("/auth/") =>
        {
            path.to_string()
        }
        _ => "/".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(exp: u64) -> Session {
        Session {
            id: "s1".into(),
            sub: "u1".into(),
            name: Some("Alex".into()),
            email: Some("alex@example.com".into()),
            exp,
        }
    }

    #[test]
    fn a_sealed_session_opens_with_the_same_key_and_purpose_only() {
        let (key, persistent) = SessionKey::from_secret(Some("s3cret"));
        assert!(persistent);
        let token = key.seal("session", &session(now() + 60));
        assert_eq!(
            key.open::<Session>("session", &token),
            Some(session(now() + 60))
        );

        // Another purpose, another key, a restart without a secret: all refused.
        assert_eq!(key.open::<Session>("login", &token), None);
        let (other, _) = SessionKey::from_secret(Some("other"));
        assert_eq!(other.open::<Session>("session", &token), None);
        let (random, persistent) = SessionKey::from_secret(None);
        assert!(!persistent);
        assert_eq!(random.open::<Session>("session", &token), None);
    }

    #[test]
    fn a_tampered_token_is_refused() {
        let (key, _) = SessionKey::from_secret(Some("s3cret"));
        let token = key.seal("session", &session(now() + 60));
        let (payload, tag) = token.split_once('.').unwrap();
        let forged = B64URL.encode(
            String::from_utf8(B64URL.decode(payload).unwrap())
                .unwrap()
                .replace("u1", "admin"),
        );
        assert_eq!(
            key.open::<Session>("session", &format!("{forged}.{tag}")),
            None
        );
        assert_eq!(key.open::<Session>("session", "garbage"), None);
        assert_eq!(key.open::<Session>("session", ""), None);
    }

    #[test]
    fn sessions_expire() {
        assert!(session(now() + 60).is_live());
        assert!(!session(now() - 1).is_live());
    }

    #[test]
    fn a_session_without_an_id_is_none() {
        let (key, _) = SessionKey::from_secret(Some("s3cret"));
        let legacy = serde_json::json!({
            "sub": "u1", "name": null, "email": null, "exp": now() + 60,
        });
        let token = key.seal("session", &legacy);
        let opened = key.open::<Session>("session", &token).unwrap();
        assert!(!opened.is_live());
    }

    #[test]
    fn return_to_stays_on_this_site() {
        assert_eq!(
            safe_return_to(Some("/connections/c1?view=queues")),
            "/connections/c1?view=queues"
        );
        for evil in [
            "https://evil.example",
            "//evil.example",
            "/\\evil.example",
            "javascript:alert(1)",
            "/ok\r\nSet-Cookie: x=y",
            "/\t/evil.example",
            "/\n/evil.example",
            "/x\\..\\evil",
            "/\u{7f}/evil.example",
            "/auth/login",
            "",
        ] {
            assert_eq!(safe_return_to(Some(evil)), "/", "{evil:?}");
        }
        assert_eq!(safe_return_to(None), "/");
    }

    #[test]
    fn reads_one_cookie_among_several() {
        let header = Some("theme=dark; ranch_session=abc.def; other=1");
        assert_eq!(cookie(header, "ranch_session"), Some("abc.def"));
        assert_eq!(cookie(header, "ranch_login"), None);
        assert_eq!(cookie(None, "ranch_session"), None);
    }

    #[test]
    fn pkce_challenge_matches_the_rfc_example() {
        // RFC 7636, appendix B.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}
