//! The server's configuration: environment variables, nothing else.
//!
//! | Variable              | Default          | Meaning                                              |
//! |-----------------------|------------------|------------------------------------------------------|
//! | `RANCH_PASSWORD`      | - (one of these  | Password of the single account (HTTP Basic auth)     |
//! | `RANCH_USERNAME`      | `ranch`          | Its user name                                        |
//! | `RANCH_OIDC_ISSUER`   |   is required)   | OpenID Connect sign-in (see below)                   |
//! | `RANCH_ALLOW_NO_AUTH` | unset            | `1` = run without any of it (trusted network only)   |
//! | `RANCH_SECRET_KEY`    | unset            | Seals the secrets file; unset = stored in clear      |
//! | `RANCH_DATA_DIR`      | `/data`          | SQLite database + secrets file                       |
//! | `RANCH_STATIC_DIR`    | `/app/dist`      | The built frontend                                   |
//! | `RANCH_BIND`          | `0.0.0.0`        | Address to listen on                                 |
//! | `RANCH_PORT`          | `8080`           | Port to listen on                                    |
//!
//! **OIDC** (Pocket ID, Authentik, Keycloak…, anything with discovery).
//! Setting `RANCH_OIDC_ISSUER` turns it on; the rest is then checked at
//! start-up, and a missing variable is named.
//!
//! | Variable                    | Default                | Meaning                                         |
//! |-----------------------------|------------------------|-------------------------------------------------|
//! | `RANCH_OIDC_ISSUER`         | -                      | Issuer URL (`…/.well-known/openid-configuration` is fetched under it) |
//! | `RANCH_OIDC_CLIENT_ID`      | - (required)           | Client id registered at the provider            |
//! | `RANCH_OIDC_CLIENT_SECRET`  | - (required)           | Its secret                                      |
//! | `RANCH_PUBLIC_URL`          | - (required)           | Where users reach Ranch; the callback is `<it>/auth/callback` |
//! | `RANCH_OIDC_SCOPES`         | `openid profile email` | (+ `groups` when groups are filtered)           |
//! | `RANCH_OIDC_ALLOWED_EMAILS` | unset                  | Comma-separated; unset = the provider decides   |
//! | `RANCH_OIDC_ALLOWED_GROUPS` | unset                  | Comma-separated (`groups` claim); either list matching is enough |
//! | `RANCH_SESSION_SECRET`      | `RANCH_SECRET_KEY`, else random | Signs the session cookie; random = sessions end on restart |
//! | `RANCH_SESSION_TTL_HOURS`   | `12`                   | How long a sign-in lasts                        |
//!
//! With OIDC on, `RANCH_PASSWORD` stays accepted as HTTP Basic: a break-glass
//! for the day the provider is down, and a way in for scripts. Leave it unset
//! to have OIDC only.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum Auth {
    Basic {
        username: String,
        password: String,
    },
    /// OpenID Connect; `basic` is the optional break-glass account.
    Oidc {
        oidc: OidcConfig,
        basic: Option<(String, String)>,
    },
    /// Explicitly asked for with `RANCH_ALLOW_NO_AUTH=1`.
    None,
}

#[derive(Debug, Clone)]
pub struct OidcConfig {
    /// Without a trailing slash.
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    /// `<RANCH_PUBLIC_URL>/auth/callback`: what is registered at the provider.
    pub redirect_url: String,
    pub scopes: String,
    /// Lower-cased.
    pub allowed_emails: Vec<String>,
    pub allowed_groups: Vec<String>,
    /// The public URL is https: cookies are `Secure`.
    pub secure_cookies: bool,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub static_dir: PathBuf,
    pub auth: Auth,
    pub secret_key: Option<String>,
    /// Signs session cookies (OIDC). `None` = a random key per process.
    pub session_secret: Option<String>,
    pub session_ttl_hours: u64,
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// `lookup` stands in for the environment, so tests need not touch it.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let get = |name: &str| lookup(name).filter(|v| !v.trim().is_empty());

        let basic = get("RANCH_PASSWORD").map(|password| {
            (
                get("RANCH_USERNAME").unwrap_or_else(|| "ranch".to_string()),
                password,
            )
        });
        let auth =
            match (Self::oidc(&get)?, basic) {
                (Some(oidc), basic) => Auth::Oidc { oidc, basic },
                (None, Some((username, password))) => Auth::Basic { username, password },
                (None, None) if get("RANCH_ALLOW_NO_AUTH").as_deref() == Some("1") => Auth::None,
                (None, None) => return Err(
                    "neither RANCH_PASSWORD nor RANCH_OIDC_ISSUER is set. Ranch can pause queues \
                     and delete jobs on the Redis it is connected to: it does not start without \
                     authentication. Set RANCH_PASSWORD (one shared account), or the \
                     RANCH_OIDC_* variables (sign-in through your identity provider), or \
                     RANCH_ALLOW_NO_AUTH=1 if, and only if, the network it listens on is \
                     trusted."
                        .to_string(),
                ),
            };

        let ip: IpAddr = get("RANCH_BIND")
            .unwrap_or_else(|| "0.0.0.0".to_string())
            .parse()
            .map_err(|e| format!("RANCH_BIND is not an IP address: {e}"))?;
        let port: u16 = get("RANCH_PORT")
            .unwrap_or_else(|| "8080".to_string())
            .parse()
            .map_err(|e| format!("RANCH_PORT is not a port: {e}"))?;

        Ok(Self {
            bind: SocketAddr::new(ip, port),
            data_dir: get("RANCH_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/data")),
            static_dir: get("RANCH_STATIC_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/app/dist")),
            auth,
            session_secret: get("RANCH_SESSION_SECRET").or_else(|| get("RANCH_SECRET_KEY")),
            session_ttl_hours: match get("RANCH_SESSION_TTL_HOURS") {
                Some(v) => v
                    .parse::<u64>()
                    .ok()
                    .filter(|h| (1..=24 * 90).contains(h))
                    .ok_or(
                        "RANCH_SESSION_TTL_HOURS must be a number of hours between 1 and 2160",
                    )?,
                None => 12,
            },
            secret_key: get("RANCH_SECRET_KEY"),
        })
    }

    /// `RANCH_OIDC_ISSUER` turns OIDC on; everything it needs is then required.
    fn oidc(get: &impl Fn(&str) -> Option<String>) -> Result<Option<OidcConfig>, String> {
        let Some(issuer) = get("RANCH_OIDC_ISSUER") else {
            return Ok(None);
        };
        let need = |name: &str| {
            get(name).ok_or_else(|| format!("RANCH_OIDC_ISSUER is set, so {name} is required"))
        };
        let client_id = need("RANCH_OIDC_CLIENT_ID")?;
        let client_secret = need("RANCH_OIDC_CLIENT_SECRET")?;
        let public_url = need("RANCH_PUBLIC_URL")?;
        for (name, url) in [
            ("RANCH_OIDC_ISSUER", &issuer),
            ("RANCH_PUBLIC_URL", &public_url),
        ] {
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(format!("{name} must be an http(s) URL, got {url:?}"));
            }
        }
        let list = |name: &str| -> Vec<String> {
            get(name)
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        };
        let allowed_groups = list("RANCH_OIDC_ALLOWED_GROUPS");
        let scopes = get("RANCH_OIDC_SCOPES").unwrap_or_else(|| {
            // Filtering on groups needs the claim: ask for it.
            if allowed_groups.is_empty() {
                "openid profile email".to_string()
            } else {
                "openid profile email groups".to_string()
            }
        });
        if !scopes.split_whitespace().any(|s| s == "openid") {
            return Err("RANCH_OIDC_SCOPES must include `openid`".to_string());
        }
        Ok(Some(OidcConfig {
            issuer: issuer.trim_end_matches('/').to_string(),
            client_id,
            client_secret,
            redirect_url: format!("{}/auth/callback", public_url.trim_end_matches('/')),
            scopes,
            allowed_emails: list("RANCH_OIDC_ALLOWED_EMAILS")
                .into_iter()
                .map(|e| e.to_lowercase())
                .collect(),
            allowed_groups,
            secure_cookies: public_url.starts_with("https://"),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(vars: &[(&str, &str)]) -> Result<ServerConfig, String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        ServerConfig::from_lookup(|name| map.get(name).cloned())
    }

    #[test]
    fn refuses_to_start_without_a_password() {
        let err = config(&[]).unwrap_err();
        assert!(err.contains("RANCH_PASSWORD") && err.contains("RANCH_OIDC_ISSUER"));
        // An empty password is no password.
        assert!(config(&[("RANCH_PASSWORD", "  ")]).is_err());
    }

    #[test]
    fn no_auth_has_to_be_asked_for_by_name() {
        assert!(matches!(
            config(&[("RANCH_ALLOW_NO_AUTH", "1")]).unwrap().auth,
            Auth::None
        ));
        assert!(config(&[("RANCH_ALLOW_NO_AUTH", "true")]).is_err());
    }

    #[test]
    fn defaults_suit_the_image() {
        let c = config(&[("RANCH_PASSWORD", "s3cret")]).unwrap();
        assert_eq!(c.bind.to_string(), "0.0.0.0:8080");
        assert_eq!(c.data_dir, PathBuf::from("/data"));
        assert_eq!(c.static_dir, PathBuf::from("/app/dist"));
        assert!(c.secret_key.is_none());
        assert!(
            matches!(c.auth, Auth::Basic { ref username, ref password } if username == "ranch" && password == "s3cret")
        );
    }

    const OIDC: [(&str, &str); 4] = [
        ("RANCH_OIDC_ISSUER", "https://id.example.com/"),
        ("RANCH_OIDC_CLIENT_ID", "ranch"),
        ("RANCH_OIDC_CLIENT_SECRET", "shh"),
        ("RANCH_PUBLIC_URL", "https://ranch.example.com/"),
    ];

    #[test]
    fn oidc_is_turned_on_by_its_issuer_and_needs_no_password() {
        let c = config(&OIDC).unwrap();
        let Auth::Oidc { oidc, basic } = c.auth else {
            panic!("expected OIDC")
        };
        assert!(basic.is_none());
        assert_eq!(oidc.issuer, "https://id.example.com");
        assert_eq!(oidc.redirect_url, "https://ranch.example.com/auth/callback");
        assert_eq!(oidc.scopes, "openid profile email");
        assert!(oidc.secure_cookies);
        assert!(oidc.allowed_emails.is_empty() && oidc.allowed_groups.is_empty());
        assert_eq!(c.session_ttl_hours, 12);
    }

    #[test]
    fn a_missing_oidc_variable_is_named() {
        for missing in [
            "RANCH_OIDC_CLIENT_ID",
            "RANCH_OIDC_CLIENT_SECRET",
            "RANCH_PUBLIC_URL",
        ] {
            let vars: Vec<_> = OIDC
                .iter()
                .copied()
                .filter(|(k, _)| *k != missing)
                .collect();
            assert!(config(&vars).unwrap_err().contains(missing), "{missing}");
        }
    }

    #[test]
    fn allow_lists_and_the_break_glass_password() {
        let mut vars = OIDC.to_vec();
        vars.extend([
            (
                "RANCH_OIDC_ALLOWED_EMAILS",
                "Alex@Example.com, ops@example.com ,",
            ),
            ("RANCH_OIDC_ALLOWED_GROUPS", "admins"),
            ("RANCH_PASSWORD", "break-glass"),
            ("RANCH_SECRET_KEY", "k"),
        ]);
        let c = config(&vars).unwrap();
        let Auth::Oidc { oidc, basic } = c.auth else {
            panic!("expected OIDC")
        };
        assert_eq!(oidc.allowed_emails, ["alex@example.com", "ops@example.com"]);
        assert_eq!(oidc.allowed_groups, ["admins"]);
        // Filtering on groups asks for the claim.
        assert_eq!(oidc.scopes, "openid profile email groups");
        assert_eq!(
            basic,
            Some(("ranch".to_string(), "break-glass".to_string()))
        );
        // The session key falls back on the secrets key: sessions survive a restart.
        assert_eq!(c.session_secret.as_deref(), Some("k"));
    }

    #[test]
    fn plain_http_is_allowed_but_cookies_are_then_not_secure() {
        let mut vars = OIDC.to_vec();
        vars[3] = ("RANCH_PUBLIC_URL", "http://ranch.lan:8080");
        let Auth::Oidc { oidc, .. } = config(&vars).unwrap().auth else {
            panic!("expected OIDC")
        };
        assert!(!oidc.secure_cookies);
        vars[3] = ("RANCH_PUBLIC_URL", "ranch.lan");
        assert!(config(&vars).unwrap_err().contains("RANCH_PUBLIC_URL"));
    }

    #[test]
    fn bad_values_are_named() {
        let err = config(&[("RANCH_PASSWORD", "x"), ("RANCH_PORT", "eighty")]).unwrap_err();
        assert!(err.contains("RANCH_PORT"));
    }
}
