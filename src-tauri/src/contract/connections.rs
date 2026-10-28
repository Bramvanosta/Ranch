//! What a connection check reports, the kinds of backend a connection may
//! name, and the endpoint of the BullMQ one.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::capabilities::CapabilityDescriptor;
use crate::domain;
use crate::domain::endpoint;
use crate::domain::ssh_config;

/// A JSON value, as TypeScript reads it: what a `serde_json::Value` field
/// is typed as on the wire (`#[specta(type = JsonValue)]`). For the
/// bindings only; nothing builds one.
#[derive(Serialize, Type)]
#[serde(untagged)]
pub enum JsonValue {
    Null(()),
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(std::collections::BTreeMap<String, JsonValue>),
}

/// One kind of backend, as the connection form offers it: its kind (what
/// a connection's `backend` holds), its label, what its endpoint asks, and
/// what every connection of the kind can do.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackendInfo {
    pub kind: String,
    pub label: String,
    pub endpoint_schema: EndpointSchema,
    pub capabilities: CapabilityDescriptor,
}

/// What a backend's endpoint holds, field by field.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EndpointSchema {
    pub fields: Vec<EndpointField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EndpointField {
    /// The key in the endpoint; for a secret, the argument it is sent in.
    pub key: String,
    pub label: String,
    pub kind: FieldKind,
    pub required: bool,
    /// What a new connection starts with.
    #[specta(type = Option<JsonValue>)]
    pub default: Option<serde_json::Value>,
}

/// How a field is entered. A `secret` is sent apart from the endpoint and
/// kept in the secret store; the `sshTunnel` group's password too.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FieldKind {
    Text,
    Number,
    Boolean,
    Secret,
    Pem,
    SshTunnel,
}

impl From<endpoint::EndpointSchema> for EndpointSchema {
    fn from(v: endpoint::EndpointSchema) -> Self {
        Self {
            fields: v
                .fields
                .into_iter()
                .map(|f| EndpointField {
                    key: f.key,
                    label: f.label,
                    kind: f.kind.into(),
                    required: f.required,
                    default: f.default,
                })
                .collect(),
        }
    }
}

impl From<endpoint::FieldKind> for FieldKind {
    fn from(v: endpoint::FieldKind) -> Self {
        match v {
            endpoint::FieldKind::Text => Self::Text,
            endpoint::FieldKind::Number => Self::Number,
            endpoint::FieldKind::Boolean => Self::Boolean,
            endpoint::FieldKind::Secret => Self::Secret,
            endpoint::FieldKind::Pem => Self::Pem,
            endpoint::FieldKind::SshTunnel => Self::SshTunnel,
        }
    }
}

/// The endpoint of a `bullmq-redis` connection, as its JSON reads: the
/// front end and the tests build and read it with this type. The BullMQ
/// backend checks it (`adapters::bullmq_redis::endpoint`). No password:
/// those are sent apart, to the secret store.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BullmqRedisEndpoint {
    pub host: String,
    pub port: u16,
    pub db: u8,
    pub username: Option<String>,
    pub tls: bool,
    /// A CA added to the system trust roots for TLS. A public certificate,
    /// not a secret.
    pub tls_custom_ca: Option<String>,
    pub ssh_tunnel: Option<SshTunnelConfig>,
}

impl BullmqRedisEndpoint {
    /// The endpoint as a connection carries it.
    pub fn json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("a Redis endpoint serializes")
    }

    /// A connection's endpoint read back, `None` when it is not this shape.
    pub fn read(endpoint: &serde_json::Value) -> Option<Self> {
        serde_json::from_value(endpoint.clone()).ok()
    }
}

/// Captured on the first successful SSH-tunneled
/// connection so the form's success banner can surface "First connect
/// to bastion: host key pinned (`<short fingerprint>`)" to the user
/// (TOFU UX). `was_new_pin == false` is silent (the keychain already
/// had the pin from an earlier connect).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SshHostKeyPin {
    pub fingerprint: String,
    pub was_new_pin: bool,
}

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestConnectionResult {
    pub success: bool,
    pub redis_version: Option<String>,
    pub error: Option<TestConnectionError>,
    /// Populated when this Test successfully established
    /// an SSH tunnel. `None` for non-SSH connections AND for failed
    /// tests (the tunnel-open error is in `error` instead).
    pub ssh_host_key_pinned: Option<SshHostKeyPin>,
    /// What the test found on a server that answered.
    #[serde(default)]
    pub probe: Option<ConnectionProbe>,
}

/// What the new-connection sheet shows once the test
/// succeeds: how fast, and what BullMQ holds there.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProbe {
    /// Round trip of one `PING`, ms.
    pub latency_ms: u32,
    /// BullMQ queues found (`…:meta` keys, bounded scan).
    pub queues_found: u32,
    /// A few of them, `prefix:name`, sorted.
    pub queue_sample: Vec<String>,
    /// The scan stopped before the end: there may be more queues.
    pub scan_capped: bool,
    /// Job schedulers across the queues found.
    pub schedulers: u32,
    /// BullMQ workers connected; `None` when the server refuses `CLIENT LIST`.
    pub workers: Option<u32>,
}

impl From<domain::probe::ProbeReport> for TestConnectionResult {
    fn from(v: domain::probe::ProbeReport) -> Self {
        Self {
            success: v.success,
            redis_version: v.version,
            error: v.error.map(Into::into),
            ssh_host_key_pinned: v.host_key.map(|k| SshHostKeyPin {
                fingerprint: k.fingerprint,
                was_new_pin: k.was_new_pin,
            }),
            probe: v.findings.map(|f| ConnectionProbe {
                latency_ms: f.latency_ms,
                queues_found: f.queues_found,
                queue_sample: f.queue_sample,
                scan_capped: f.scan_capped,
                schedulers: f.schedulers,
                workers: f.workers,
            }),
        }
    }
}

/// Result of a `ping_connection` call. Same success-encoded-in-payload pattern
/// as `TestConnectionResult` (errors live in `error: Option<TestConnectionError>`,
/// not in the outer `Result`). The `Result<_, RanchErrorDto>` outer wrapper is
/// reserved for system errors (DB read failed, keychain unavailable, connection
/// not found).
#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PingConnectionResult {
    pub success: bool,
    pub latency_ms: Option<u32>,
    pub error: Option<TestConnectionError>,
}

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TestConnectionError {
    AuthFailed,
    CannotReach {
        reason: String,
    },
    Timeout,
    TlsHandshake {
        reason: String,
    },
    /// The SSH tunnel step failed before Redis was reached.
    /// The form's `renderTestError` switch renders `reason` verbatim.
    /// Three distinct backend variants (`SshAuthFailed`,
    /// `SshKeyFileError`, `SshTunnelFailed`) collapse to this single
    /// form-facing arm; the form doesn't branch on the SSH sub-kind.
    SshTunnel {
        reason: String,
    },
    Other {
        reason: String,
    },
}

impl From<domain::probe::PingReport> for PingConnectionResult {
    fn from(v: domain::probe::PingReport) -> Self {
        Self {
            success: v.success,
            latency_ms: v.latency_ms,
            error: v.error.map(Into::into),
        }
    }
}

impl From<domain::probe::ProbeError> for TestConnectionError {
    fn from(v: domain::probe::ProbeError) -> Self {
        use domain::probe::ProbeError as E;
        match v {
            E::AuthFailed => Self::AuthFailed,
            E::CannotReach { reason } => Self::CannotReach { reason },
            E::Timeout => Self::Timeout,
            E::TlsHandshake { reason } => Self::TlsHandshake { reason },
            E::SshTunnel { reason } => Self::SshTunnel { reason },
            E::Other { reason } => Self::Other { reason },
        }
    }
}

/// Per-connection read-only state.
///
/// Initialized from the `connections.is_locked` column at actor-spawn
/// time. In-memory authoritative once the actor is alive; a restart
/// re-spawns the actor and re-reads `is_locked`, so unlocks do NOT
/// leak across launches.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReadOnlyState {
    Locked,
    Unlocked,
}

impl From<domain::lock::ReadOnlyState> for ReadOnlyState {
    fn from(v: domain::lock::ReadOnlyState) -> Self {
        match v {
            domain::lock::ReadOnlyState::Locked => Self::Locked,
            domain::lock::ReadOnlyState::Unlocked => Self::Unlocked,
        }
    }
}

/// Read-shape exposed to the frontend. The structural SSH tunnel config
/// (host / port / user / auth method / optional jump host): no password.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SshTunnelConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuthMethod,
    pub jump_host: Option<SshJumpHost>,
}

impl From<ssh_config::SshTunnelConfig> for SshTunnelConfig {
    fn from(v: ssh_config::SshTunnelConfig) -> Self {
        Self {
            host: v.host,
            port: v.port,
            user: v.user,
            auth: v.auth.into(),
            jump_host: v.jump_host.map(Into::into),
        }
    }
}
impl From<SshTunnelConfig> for ssh_config::SshTunnelConfig {
    fn from(v: SshTunnelConfig) -> Self {
        Self {
            host: v.host,
            port: v.port,
            user: v.user,
            auth: v.auth.into(),
            jump_host: v.jump_host.map(Into::into),
        }
    }
}

/// Discriminated auth-method union.
///
/// `KeyFile` carries a path string; we do NOT read or validate the file
/// contents here (the BullMQ backend's `ssh_tunnel` does at connect time). `Password` is a marker
/// variant: the actual password text lives in the keychain at
/// `{connection_id}::ssh`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SshAuthMethod {
    KeyFile { key_file_path: String },
    Password,
}

impl From<ssh_config::SshAuthMethod> for SshAuthMethod {
    fn from(v: ssh_config::SshAuthMethod) -> Self {
        match v {
            ssh_config::SshAuthMethod::KeyFile { key_file_path } => Self::KeyFile { key_file_path },
            ssh_config::SshAuthMethod::Password => Self::Password,
        }
    }
}

impl From<SshAuthMethod> for ssh_config::SshAuthMethod {
    fn from(v: SshAuthMethod) -> Self {
        match v {
            SshAuthMethod::KeyFile { key_file_path } => Self::KeyFile { key_file_path },
            SshAuthMethod::Password => Self::Password,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SshJumpHost {
    pub host: String,
    pub port: u16,
    pub user: String,
}

impl From<ssh_config::SshJumpHost> for SshJumpHost {
    fn from(v: ssh_config::SshJumpHost) -> Self {
        Self {
            host: v.host,
            port: v.port,
            user: v.user,
        }
    }
}
impl From<SshJumpHost> for ssh_config::SshJumpHost {
    fn from(v: SshJumpHost) -> Self {
        Self {
            host: v.host,
            port: v.port,
            user: v.user,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::bullmq_redis::endpoint::{BullmqRedisSpec, RedisEndpoint};
    use crate::application::ports::BackendSpec;

    /// The wire's endpoint type and the backend's read the same JSON.
    #[test]
    fn the_bullmq_endpoint_reads_as_the_backend_reads_it() {
        let wire = BullmqRedisEndpoint {
            host: "redis.lumen.example".into(),
            port: 6380,
            db: 2,
            username: Some("app".into()),
            tls: true,
            tls_custom_ca: None,
            ssh_tunnel: Some(SshTunnelConfig {
                host: "bastion".into(),
                port: 22,
                user: "ops".into(),
                auth: SshAuthMethod::KeyFile {
                    key_file_path: "~/.ssh/id".into(),
                },
                jump_host: None,
            }),
        };
        let stored = BullmqRedisSpec
            .validate(&endpoint::Endpoint::new(wire.json()))
            .unwrap();
        assert_eq!(stored.as_json(), &wire.json());
        let read = RedisEndpoint::parse(&stored).unwrap();
        assert_eq!(
            (read.host.as_str(), read.port, read.db),
            ("redis.lumen.example", 6380, 2)
        );
        let back: BullmqRedisEndpoint = serde_json::from_value(stored.into_json()).unwrap();
        assert_eq!(back, wire);
    }

    #[test]
    fn read_only_state_round_trips_through_camel_case() {
        // `Locked` / `Unlocked` serialize lowercase first letter via
        // serde's camelCase rename rule. Confirm the wire shape matches
        // what the frontend will see.
        let locked =
            serde_json::to_string(&ReadOnlyState::from(domain::lock::ReadOnlyState::Locked))
                .unwrap();
        assert_eq!(locked, "\"locked\"");
        let unlocked =
            serde_json::to_string(&ReadOnlyState::from(domain::lock::ReadOnlyState::Unlocked))
                .unwrap();
        assert_eq!(unlocked, "\"unlocked\"");

        let back: ReadOnlyState = serde_json::from_str("\"locked\"").unwrap();
        assert_eq!(back, ReadOnlyState::Locked);
        let back: ReadOnlyState = serde_json::from_str("\"unlocked\"").unwrap();
        assert_eq!(back, ReadOnlyState::Unlocked);
    }
}
