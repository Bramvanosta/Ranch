//! The endpoint of a BullMQ-on-Redis connection: the Redis server, its
//! database, the user, TLS with an optional CA, and an optional SSH tunnel.
//!
//! Stored as a JSON object in `connections.endpoint` and read back into
//! [`RedisEndpoint`] here, and only here: the connection actor and the
//! probe read the typed value, never the JSON. The password and
//! the SSH password are secrets, kept in the secret store.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::tls::validate_ca_pem;
use ranch_application::ports::SecretStore;
use ranch_application::ports::{BackendSpec, Capabilities};
use ranch_domain::endpoint::{BackendKind, Endpoint, EndpointField, EndpointSchema, FieldKind};
use ranch_domain::errors::RanchError;
use ranch_domain::probe::{ProbeReport, ProbeSecrets};
use ranch_domain::ssh_config::{validate_ssh_tunnel_config, SshTunnelConfig};

/// The kind a BullMQ-on-Redis connection names.
pub const BULLMQ_REDIS: &str = "bullmq-redis";

/// Where a BullMQ-on-Redis connection points. Every key is written, an
/// absent value as `null`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RedisEndpoint {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub db: u8,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub tls: bool,
    /// A CA added to the system trust roots for TLS. A public
    /// certificate, not a secret.
    #[serde(default)]
    pub tls_custom_ca: Option<String>,
    /// The bastion the connection goes through. Its password, when it
    /// uses one, is in the secret store at `{id}::ssh`.
    #[serde(default)]
    pub ssh_tunnel: Option<SshTunnelConfig>,
}

impl RedisEndpoint {
    /// Redis at `host:port`, database 0, no user, TLS or tunnel.
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            db: 0,
            username: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
        }
    }

    /// Read an endpoint as stored or sent. Says what is wrong with it.
    pub fn parse(endpoint: &Endpoint) -> Result<Self, RanchError> {
        serde_json::from_value(endpoint.as_json().clone()).map_err(|e| RanchError::InvalidInput {
            reason: format!("Invalid Redis endpoint: {e}"),
        })
    }

    pub fn to_endpoint(&self) -> Endpoint {
        Endpoint::new(serde_json::to_value(self).expect("a Redis endpoint serializes"))
    }

    /// The URL to connect with. Built with `url::Url`, which percent-encodes
    /// the user name and password; something it refuses (an empty host)
    /// stays the bare URL, and the Redis client reports it.
    pub fn url(&self, password: Option<&str>) -> String {
        let scheme = if self.tls { "rediss" } else { "redis" };
        with_credentials(
            format!("{scheme}://{}:{}/{}", self.host, self.port, self.db),
            self.username.as_deref(),
            password,
        )
    }

    /// The URL through an SSH tunnel's local listener: always `redis://`,
    /// since the SSH session is the encryption layer (TLS on the loopback
    /// hop would only fail on the certificate's name). User, password and
    /// database are kept.
    pub fn url_via(&self, local: std::net::SocketAddr, password: Option<&str>) -> String {
        with_credentials(
            format!("redis://{}:{}/{}", local.ip(), local.port(), self.db),
            self.username.as_deref(),
            password,
        )
    }

    /// The server as an agent reads it: `redis://host:port/db`, `rediss://`
    /// with TLS, "via SSH" through a tunnel. No user name, no password.
    pub fn describe(&self) -> String {
        let scheme = if self.tls { "rediss" } else { "redis" };
        let via = if self.ssh_tunnel.is_some() {
            " via SSH"
        } else {
            ""
        };
        format!("{scheme}://{}:{}/{}{via}", self.host, self.port, self.db)
    }
}

fn with_credentials(bare: String, username: Option<&str>, password: Option<&str>) -> String {
    let Ok(mut u) = url::Url::parse(&bare) else {
        return bare;
    };
    if let Some(user) = username {
        let _ = u.set_username(user);
    }
    if let Some(pass) = password {
        let _ = u.set_password(Some(pass));
    }
    u.to_string()
}

/// The `bullmq-redis` kind: what it can do, its form, its endpoint
/// checked, and the Redis behind one probed.
pub struct BullmqRedisSpec;

#[async_trait]
impl BackendSpec for BullmqRedisSpec {
    fn kind(&self) -> BackendKind {
        BackendKind::new(BULLMQ_REDIS)
    }

    fn label(&self) -> &str {
        "BullMQ on Redis"
    }

    fn capabilities(&self) -> &Capabilities {
        &super::CAPABILITIES
    }

    fn endpoint_schema(&self) -> EndpointSchema {
        let field = |key: &str, label: &str, kind, required, default| EndpointField {
            key: key.to_string(),
            label: label.to_string(),
            kind,
            required,
            default,
        };
        EndpointSchema {
            fields: vec![
                field("host", "Host", FieldKind::Text, true, None),
                field("port", "Port", FieldKind::Number, true, Some(json!(6379))),
                field("db", "DB index", FieldKind::Number, true, Some(json!(0))),
                field("username", "Username", FieldKind::Text, false, None),
                field("password", "Password", FieldKind::Secret, false, None),
                field(
                    "tls",
                    "Use TLS",
                    FieldKind::Boolean,
                    false,
                    Some(json!(false)),
                ),
                field(
                    "tlsCustomCa",
                    "Custom CA certificate",
                    FieldKind::Pem,
                    false,
                    None,
                ),
                field("sshTunnel", "SSH tunnel", FieldKind::SshTunnel, false, None),
            ],
        }
    }

    fn validate(&self, endpoint: &Endpoint) -> Result<Endpoint, RanchError> {
        let mut ep = RedisEndpoint::parse(endpoint)?;
        if ep.host.trim().is_empty() {
            return Err(RanchError::InvalidInput {
                reason: "Host is required".to_string(),
            });
        }
        // The user name goes out byte for byte in `AUTH`: a line break in
        // it would add commands of the sender's choosing.
        if ep
            .username
            .as_deref()
            .is_some_and(|u| u.chars().any(char::is_control))
        {
            return Err(RanchError::InvalidInput {
                reason: "The user name cannot contain control characters (line breaks, tabs)"
                    .to_string(),
            });
        }
        ep.tls_custom_ca = validate_ca_pem(ep.tls_custom_ca.as_deref())?;
        if let Some(tunnel) = &ep.ssh_tunnel {
            validate_ssh_tunnel_config(tunnel)?;
        }
        Ok(ep.to_endpoint())
    }

    fn same_server(&self, saved: &Endpoint, new: &Endpoint) -> bool {
        match (RedisEndpoint::parse(saved), RedisEndpoint::parse(new)) {
            // Host (case-insensitive), port, and TLS: a saved password
            // never goes out in clear to a server that had TLS.
            (Ok(a), Ok(b)) => {
                a.host.eq_ignore_ascii_case(b.host.trim()) && a.port == b.port && a.tls == b.tls
            }
            _ => false,
        }
    }

    fn tunnel(&self, endpoint: &Endpoint) -> Option<SshTunnelConfig> {
        RedisEndpoint::parse(endpoint).ok()?.ssh_tunnel
    }

    fn describe(&self, endpoint: &Endpoint) -> String {
        match RedisEndpoint::parse(endpoint) {
            Ok(ep) => ep.describe(),
            Err(_) => "an unreadable Redis endpoint".to_string(),
        }
    }

    async fn probe(
        &self,
        endpoint: &Endpoint,
        secrets: &ProbeSecrets,
        keychain: Arc<dyn SecretStore>,
    ) -> Result<ProbeReport, RanchError> {
        super::probe::probe(&RedisEndpoint::parse(endpoint)?, secrets, keychain).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ranch_domain::ssh_config::SshAuthMethod;

    fn endpoint(v: serde_json::Value) -> Endpoint {
        Endpoint::new(v)
    }

    fn tunnel() -> SshTunnelConfig {
        SshTunnelConfig {
            host: "bastion.example.com".into(),
            port: 22,
            user: "ubuntu".into(),
            auth: SshAuthMethod::Password,
            jump_host: None,
        }
    }

    #[test]
    fn an_endpoint_is_stored_with_every_key() {
        let stored = BullmqRedisSpec
            .validate(&endpoint(json!({ "host": "h", "port": 6379 })))
            .unwrap();
        assert_eq!(
            stored.as_json(),
            &json!({
                "host": "h", "port": 6379, "db": 0, "username": null, "tls": false,
                "tlsCustomCa": null, "sshTunnel": null
            })
        );
    }

    #[test]
    fn a_blank_ca_is_no_ca_and_a_tunnel_is_checked() {
        let stored = BullmqRedisSpec
            .validate(&endpoint(
                json!({ "host": "h", "port": 1, "tlsCustomCa": "  " }),
            ))
            .unwrap();
        assert_eq!(RedisEndpoint::parse(&stored).unwrap().tls_custom_ca, None);

        let mut bad = tunnel();
        bad.user = " ".into();
        let err = BullmqRedisSpec
            .validate(
                &RedisEndpoint {
                    ssh_tunnel: Some(bad),
                    ..RedisEndpoint::new("h", 1)
                }
                .to_endpoint(),
            )
            .unwrap_err();
        assert!(err.to_string().contains("SSH user"), "{err}");
    }

    #[test]
    fn what_is_not_an_endpoint_is_refused() {
        for bad in [
            json!({ "host": "h" }),
            json!({ "host": "", "port": 1 }),
            json!({ "host": "h", "port": 70000 }),
            json!({ "host": "h", "port": 1, "user": "x" }),
            json!({ "host": "h", "port": 1, "username": "a\r\nFLUSHALL" }),
            json!({ "host": "h", "port": 1, "tlsCustomCa": "not a pem" }),
            json!("redis://h:1"),
        ] {
            assert!(
                BullmqRedisSpec.validate(&endpoint(bad.clone())).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_saved_secret_follows_host_port_and_tls() {
        let saved = endpoint(json!({ "host": "Redis.example.com", "port": 6379, "tls": true }));
        let same =
            endpoint(json!({ "host": " redis.example.com ", "port": 6379, "tls": true, "db": 3 }));
        assert!(BullmqRedisSpec.same_server(&saved, &same));
        for moved in [
            json!({ "host": "evil.example.com", "port": 6379, "tls": true }),
            json!({ "host": "redis.example.com", "port": 6380, "tls": true }),
            json!({ "host": "redis.example.com", "port": 6379, "tls": false }),
            json!({ "port": 6379 }),
        ] {
            assert!(
                !BullmqRedisSpec.same_server(&saved, &endpoint(moved.clone())),
                "{moved}"
            );
        }
    }

    #[test]
    fn the_url_carries_the_credentials_percent_encoded() {
        let mut ep = RedisEndpoint::new("h", 6379);
        assert_eq!(ep.url(None), "redis://h:6379/0");
        assert_eq!(ep.url(Some("s")), "redis://:s@h:6379/0");
        ep.username = Some("u".into());
        ep.tls = true;
        ep.db = 3;
        assert_eq!(ep.url(Some("p@ss")), "rediss://u:p%40ss@h:6379/3");
        let local: std::net::SocketAddr = "127.0.0.1:50000".parse().unwrap();
        assert_eq!(
            ep.url_via(local, Some("s")),
            "redis://u:s@127.0.0.1:50000/3"
        );
    }

    #[test]
    fn an_agent_reads_where_it_points_without_credentials() {
        let mut ep = RedisEndpoint::parse(&endpoint(
            json!({ "host": "h", "port": 6380, "db": 2, "username": "admin", "tls": true }),
        ))
        .unwrap();
        assert_eq!(ep.describe(), "rediss://h:6380/2");
        ep.tls = false;
        ep.ssh_tunnel = Some(tunnel());
        assert_eq!(
            BullmqRedisSpec.describe(&ep.to_endpoint()),
            "redis://h:6380/2 via SSH"
        );
        assert_eq!(BullmqRedisSpec.tunnel(&ep.to_endpoint()), Some(tunnel()));
    }
}
