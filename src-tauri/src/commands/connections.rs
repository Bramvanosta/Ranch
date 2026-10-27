//! Connection commands.
//!
//! The four commands here form the first end-to-end feature surface in Ranch:
//!
//! 1. [`parse_redis_url`] — pure URL → struct (no IO). Powers the form's
//!    paste-and-fill UX.
//! 2. [`test_connection`] — open a one-shot async Redis connection, PING +
//!    INFO server, return a structured outcome (success+version OR a
//!    classified error). Hard-bounded at 2 s.
//! 3. [`add_connection`] — persist a connection: SQLite row + (optional)
//!    keychain entry, with transactional rollback if either side fails.
//! 4. [`list_connections`] — read joined connection rows for the sidebar.
//!
//! Architecture invariants enforced here:
//! - All Redis IO goes through these commands; no direct `redis-rs` use elsewhere.
//! - Credentials never land in SQLite — the `keychain_ref` column points at
//!   the keychain entry whose name is the connection id.
//! - Commands 3 + 4 split a thin Tauri wrapper from a `_inner` async fn that
//!   takes `&SqlitePool, &dyn KeychainAdapter` so they're testable without
//!   spinning up a Tauri runtime.

use std::sync::Arc;

use crate::app_state::AppState;
use crate::commands::hierarchy::{resolve_hierarchy, EnvironmentSelector, GroupSelector};
use crate::core::connection_actor::{ConnectionConfig, ConnectionRegistry};
use crate::core::errors::{RanchError, RanchErrorDto};
use crate::core::keychain::KeychainAdapter;
use crate::core::safety_lock::ReadOnlyState;
use crate::core::ssh_config::{
    self, decode_from_json, encode_to_json, validate_ssh_tunnel_config, SshAuthMethod,
    SshTunnelConfig,
};
use crate::core::tls::{build_redis_client, validate_ca_pem};
use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::SqlitePool;
use url::Url;

/// Keychain entry name for the SSH password. Distinct from the
/// Redis password (stored at `{connection_id}`) so
/// both can coexist on a single connection.
fn ssh_keychain_name(connection_id: &str) -> String {
    format!("{connection_id}::ssh")
}

/// Keychain entry name for the per-connection SSH
/// host-key pin (TOFU model). Distinct from `{id}` (Redis password)
/// and `{id}::ssh` (SSH password). `remove_connection` cleans this
/// third entry; `duplicate_connection` does NOT carry it (each
/// duplicate re-pins on first connect).
pub fn ssh_hostkey_keychain_name(connection_id: &str) -> String {
    format!("{connection_id}::ssh::hostkey")
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. parse_redis_url
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ParsedRedisUrl {
    pub host: String,
    pub port: u16,
    pub db: u8,
    pub username: Option<String>,
    pub password: Option<String>,
    pub tls: bool,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn parse_redis_url(url: String) -> Result<ParsedRedisUrl, RanchErrorDto> {
    parse_redis_url_inner(url).await.map_err(Into::into)
}

pub async fn parse_redis_url_inner(url: String) -> Result<ParsedRedisUrl, RanchError> {
    let parsed = Url::parse(&url).map_err(|e| RanchError::InvalidRedisUrl {
        reason: format!("invalid URL: {e}"),
    })?;

    let scheme = parsed.scheme();
    let tls = match scheme {
        "redis" => false,
        "rediss" => true,
        other => {
            return Err(RanchError::InvalidRedisUrl {
                reason: format!("unsupported scheme `{other}`; expected `redis` or `rediss`"),
            })
        }
    };

    let host = parsed
        .host_str()
        .ok_or_else(|| RanchError::InvalidRedisUrl {
            reason: "missing host".to_string(),
        })?
        .to_string();

    if host.is_empty() {
        return Err(RanchError::InvalidRedisUrl {
            reason: "missing host".to_string(),
        });
    }

    let port = parsed.port().unwrap_or(6379);

    // /0, /1, ... — default 0 if path is empty or non-numeric.
    let db: u8 = parsed.path().trim_start_matches('/').parse().unwrap_or(0);

    let username = if parsed.username().is_empty() {
        None
    } else {
        Some(parsed.username().to_string())
    };

    // url::Url percent-encodes user/password — decode for display.
    let password = parsed.password().map(|p| {
        percent_encoding::percent_decode_str(p)
            .decode_utf8_lossy()
            .into_owned()
    });

    Ok(ParsedRedisUrl {
        host,
        port,
        db,
        username,
        password,
        tls,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. test_connection
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestConnectionArgs {
    pub host: String,
    pub port: u16,
    pub db: u8,
    pub username: Option<String>,
    pub password: Option<String>,
    pub tls: bool,
    /// Optional CA PEM to add to rustls's trust roots.
    /// `None` / `Some("")` / whitespace-only → treated as "no custom CA"
    /// (system trust roots only). Only meaningful when `tls = true`.
    /// Validated by `core::tls::validate_ca_pem` before client construction.
    pub tls_custom_ca: Option<String>,
    /// When present, `test_connection` activates the SSH
    /// tunnel runtime for the Test call: opens the
    /// bastion session, pings Redis through the forwarded channel,
    /// then disposes the tunnel. `None` means a direct connect.
    pub ssh_tunnel: Option<SshTunnelConfig>,
    /// SSH password for the Test call when
    /// `ssh_tunnel.auth.kind == "password"`. The form passes the
    /// current draft value (which is empty in edit mode unless the
    /// user typed a new password — same UX as the Redis password).
    /// `None` falls back to key-file auth or anonymous attempt.
    #[serde(default)]
    pub ssh_password: Option<String>,
    /// Editing a saved connection: a blank password (Redis or
    /// SSH) means "the one already in the keychain", as it does on save, so
    /// the test does not fail for a password the form never shows.
    #[serde(default)]
    pub saved_connection_id: Option<String>,
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
    /// form-facing arm — the form doesn't branch on the SSH sub-kind.
    SshTunnel {
        reason: String,
    },
    Other {
        reason: String,
    },
}

/// Captured on the first successful SSH-tunneled
/// connection so the form's success banner can surface "First connect
/// to bastion — host key pinned (`<short fingerprint>`)" to the user
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

/// Bounded discovery for the test: a few `SCAN` rounds, a sample of names.
const PROBE_SCAN_ROUNDS: usize = 10;
const PROBE_SAMPLE: usize = 5;

/// Collapse the three SSH RanchError variants into the
/// single `TestConnectionError::SshTunnel` arm used by the form. The
/// form's `renderTestError` switch displays the reason verbatim — no
/// need to fan out three arms there.
pub(crate) fn ssh_ranch_err_to_test_err(e: &RanchError) -> Option<TestConnectionError> {
    match e {
        RanchError::SshAuthFailed { reason } => Some(TestConnectionError::SshTunnel {
            reason: reason.clone(),
        }),
        RanchError::SshKeyFileError { path, reason } => Some(TestConnectionError::SshTunnel {
            reason: format!("Key file at {path}: {reason}"),
        }),
        RanchError::SshTunnelFailed { reason } => Some(TestConnectionError::SshTunnel {
            reason: reason.clone(),
        }),
        _ => None,
    }
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn test_connection(
    args: TestConnectionArgs,
) -> Result<TestConnectionResult, RanchErrorDto> {
    test_connection_inner(args).await.map_err(Into::into)
}

/// Fill a blank password from the keychain of the connection
/// being edited (keychain ref = connection id). A keychain that cannot be
/// read leaves it blank: the test then reports what Redis says.
fn with_saved_secrets(
    mut args: TestConnectionArgs,
    keychain: &dyn KeychainAdapter,
) -> TestConnectionArgs {
    let Some(id) = args.saved_connection_id.clone() else {
        return args;
    };
    if args.password.is_none() {
        args.password = keychain.get(&id).unwrap_or(None);
    }
    let ssh_password_auth = matches!(
        args.ssh_tunnel.as_ref().map(|c| &c.auth),
        Some(SshAuthMethod::Password)
    );
    if ssh_password_auth && args.ssh_password.is_none() {
        args.ssh_password = keychain.get(&ssh_keychain_name(&id)).unwrap_or(None);
    }
    args
}

/// Desktop only: it reaches for the OS keychain. The HTTP shell calls
/// [`test_connection_inner_with_keychain`] with its own secret store.
#[cfg(feature = "desktop")]
pub async fn test_connection_inner(
    args: TestConnectionArgs,
) -> Result<TestConnectionResult, RanchError> {
    test_connection_inner_with_keychain(args, default_keychain()).await
}

/// Default keychain for Tauri-command callers. Tests inject their own
/// via [`test_connection_inner_with_keychain`].
#[cfg(feature = "desktop")]
fn default_keychain() -> Arc<dyn KeychainAdapter> {
    Arc::new(crate::core::keychain::OsKeychain::new())
}

/// Testable variant of `test_connection_inner` that takes
/// an explicit keychain handle. The SSH tunnel layer needs a keychain
/// for the TOFU host-key pin (see `core::ssh_tunnel`); production
/// callers use the OS keychain, integration tests inject an in-memory
/// one.
pub async fn test_connection_inner_with_keychain(
    args: TestConnectionArgs,
    keychain: Arc<dyn KeychainAdapter>,
) -> Result<TestConnectionResult, RanchError> {
    // Validate the CA PEM up-front so a malformed paste surfaces as a typed
    // RanchError::InvalidInput rather than a cryptic mid-handshake failure
    //.
    let validated_ca = validate_ca_pem(args.tls_custom_ca.as_deref())?;
    let args = with_saved_secrets(args, keychain.as_ref());

    // When an SSH tunnel is configured, open it once for
    // this Test, route the Redis PING through it, then drop it. We do
    // NOT cache anything here: Test is exploratory + one-shot. The
    // bulk of SSH-related latency lives in this branch (cold open).
    if let Some(ssh_cfg) = args.ssh_tunnel.as_ref() {
        // Use a synthetic connection id for the keychain hostkey pin.
        // The Test path doesn't know the eventual connection_id (the
        // user is still filling the form), so we key on a stable
        // "test::{host}:{port}" name. Real connects use the real id
        // once the connection is saved.
        let synthetic_id = format!("test::{}:{}", ssh_cfg.host, ssh_cfg.port);
        let open_result = crate::core::ssh_tunnel::open(
            ssh_cfg,
            args.ssh_password.as_deref(),
            (&args.host, args.port),
            keychain,
            &synthetic_id,
        )
        .await;
        match open_result {
            Ok(opened) => {
                let local = opened.tunnel.local_addr();
                let url = build_redis_url_for_local_loopback(&args, local);
                let result = tokio::time::timeout(
                    std::time::Duration::from_millis(20_000),
                    ping_and_version(&url, None),
                )
                .await;
                let pin = Some(SshHostKeyPin {
                    fingerprint: opened.fingerprint,
                    was_new_pin: opened.was_new_pin,
                });
                // tunnel drops at end-of-scope; one-shot Test.
                drop(opened.tunnel);
                return Ok(match result {
                    Ok(Ok((version, probe))) => TestConnectionResult {
                        success: true,
                        redis_version: version,
                        error: None,
                        ssh_host_key_pinned: pin,
                        probe: Some(probe),
                    },
                    Ok(Err(e)) => TestConnectionResult {
                        success: false,
                        redis_version: None,
                        error: Some(map_redis_err(e)),
                        ssh_host_key_pinned: pin,
                        probe: None,
                    },
                    Err(_elapsed) => TestConnectionResult {
                        success: false,
                        redis_version: None,
                        error: Some(TestConnectionError::Timeout),
                        ssh_host_key_pinned: pin,
                        probe: None,
                    },
                });
            }
            Err(err) => {
                if let Some(test_err) = ssh_ranch_err_to_test_err(&err) {
                    return Ok(TestConnectionResult {
                        success: false,
                        redis_version: None,
                        error: Some(test_err),
                        ssh_host_key_pinned: None,
                        probe: None,
                    });
                }
                return Err(err);
            }
        }
    }

    let url = build_redis_url(&args);

    let result = tokio::time::timeout(
        // The test also looks at what BullMQ holds: a little longer.
        // And a link of up to ~1 s round trip is slow, not down.
        std::time::Duration::from_millis(15_000),
        ping_and_version(&url, validated_ca.as_deref()),
    )
    .await;

    Ok(match result {
        Ok(Ok((version, probe))) => TestConnectionResult {
            success: true,
            redis_version: version,
            error: None,
            ssh_host_key_pinned: None,
            probe: Some(probe),
        },
        Ok(Err(e)) => TestConnectionResult {
            success: false,
            redis_version: None,
            error: Some(map_redis_err(e)),
            ssh_host_key_pinned: None,
            probe: None,
        },
        Err(_elapsed) => TestConnectionResult {
            success: false,
            redis_version: None,
            error: Some(TestConnectionError::Timeout),
            ssh_host_key_pinned: None,
            probe: None,
        },
    })
}

/// Build the Redis URL targeting the SSH-tunnel's local
/// loopback listener. Username + password + db preserved from `args`;
/// TLS is explicitly OFF on the loopback hop (the SSH session is the
/// encryption layer; double-encrypting localhost only invites
/// cert-name-mismatch errors).
fn build_redis_url_for_local_loopback(
    args: &TestConnectionArgs,
    local: std::net::SocketAddr,
) -> String {
    let bare = format!("redis://{}:{}/{}", local.ip(), local.port(), args.db);
    let mut u = match Url::parse(&bare) {
        Ok(u) => u,
        Err(_) => return bare,
    };
    if let Some(user) = args.username.as_deref() {
        let _ = u.set_username(user);
    }
    if let Some(pass) = args.password.as_deref() {
        let _ = u.set_password(Some(pass));
    }
    u.to_string()
}

async fn ping_and_version(
    url: &str,
    tls_custom_ca: Option<&str>,
) -> Result<(Option<String>, ConnectionProbe), redis::RedisError> {
    let client = build_redis_client(url, tls_custom_ca)?;
    let mut conn = client
        .get_multiplexed_async_connection_with_config(&crate::core::tls::async_connection_config())
        .await?;
    let started = std::time::Instant::now();
    let _: String = redis::cmd("PING").query_async(&mut conn).await?;
    let latency_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
    let info: String = redis::cmd("INFO")
        .arg("server")
        .query_async(&mut conn)
        .await?;
    let version = info
        .lines()
        .find_map(|l| l.strip_prefix("redis_version:"))
        .map(|v| v.trim().to_string());
    // What BullMQ holds there. Best effort: a refused command
    // leaves its part empty, never fails the test.
    let mut probe = ConnectionProbe {
        latency_ms,
        ..Default::default()
    };
    let mut queues: Vec<String> = Vec::new();
    let mut cursor: u64 = 0;
    for _ in 0..PROBE_SCAN_ROUNDS {
        let step: Result<(u64, Vec<String>), _> = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("*:meta")
            .arg("COUNT")
            .arg(1000)
            .query_async(&mut conn)
            .await;
        let Ok((next, keys)) = step else { break };
        queues.extend(
            keys.into_iter()
                .filter_map(|k| k.strip_suffix(":meta").map(str::to_string)),
        );
        cursor = next;
        if cursor == 0 {
            break;
        }
    }
    probe.scan_capped = cursor != 0;
    queues.sort();
    queues.dedup();
    probe.queues_found = queues.len() as u32;
    probe.queue_sample = queues.iter().take(PROBE_SAMPLE).cloned().collect();
    if !queues.is_empty() {
        let mut pipe = redis::pipe();
        for q in queues.iter().take(200) {
            pipe.cmd("ZCARD").arg(format!("{q}:repeat"));
        }
        let counts: Result<Vec<u64>, _> = pipe.query_async(&mut conn).await;
        probe.schedulers = counts.map(|c| c.iter().sum::<u64>() as u32).unwrap_or(0);
    }
    let list: Result<String, _> = redis::cmd("CLIENT")
        .arg("LIST")
        .query_async(&mut conn)
        .await;
    probe.workers = list.ok().map(|l| {
        let refs: Vec<crate::core::bullmq_schema::QueueRef> = queues
            .iter()
            .filter_map(|q| {
                q.split_once(':')
                    .map(|(prefix, name)| crate::core::bullmq_schema::QueueRef {
                        prefix: prefix.to_string(),
                        name: name.to_string(),
                    })
            })
            .collect();
        crate::core::bullmq_schema::workers::count_workers(&l, &refs)
            .iter()
            .map(|c| c.workers)
            .sum()
    });
    Ok((version, probe))
}

fn build_redis_url(args: &TestConnectionArgs) -> String {
    // Build via `url::Url` so the crate handles percent-encoding of any
    // special characters in username/password. Constructing the bare URL first
    // and then calling `set_username` / `set_password` is the canonical pattern.
    let scheme = if args.tls { "rediss" } else { "redis" };
    let bare = format!(
        "{scheme}://{host}:{port}/{db}",
        host = args.host,
        port = args.port,
        db = args.db
    );
    let mut u = match Url::parse(&bare) {
        Ok(u) => u,
        // Fall back to the bare string if the user typed something url::Url
        // refuses (e.g. an empty host) — `redis::Client::open` will then surface
        // the error through the regular failure path.
        Err(_) => return bare,
    };
    if let Some(user) = args.username.as_deref() {
        let _ = u.set_username(user);
    }
    if let Some(pass) = args.password.as_deref() {
        let _ = u.set_password(Some(pass));
    }
    u.to_string()
}

/// Substrings used to detect TLS-related failures in redis-rs error messages.
/// rustls handshake errors come through as `ErrorKind::IoError` on most
/// platforms; we rely on the message text for finer granularity. The list
/// covers the common rustls verification phrases.
const TLS_NEEDLES: &[&str] = &[
    "tls",
    "certificate",
    "handshake",
    "verify",
    "unknown ca",
    "self signed",
    "self-signed",
    "expired",
];

fn map_redis_err(e: redis::RedisError) -> TestConnectionError {
    use redis::ErrorKind;
    let kind = e.kind();
    let msg = e.to_string();
    let lower = msg.to_lowercase();

    if TLS_NEEDLES.iter().any(|needle| lower.contains(needle)) {
        return TestConnectionError::TlsHandshake { reason: msg };
    }

    match kind {
        ErrorKind::AuthenticationFailed => TestConnectionError::AuthFailed,
        // redis-rs 1.x renamed `IoError` → `Io`. This catches platform-level
        // network failures (refused, no route to host, dns failure, etc.).
        ErrorKind::Io => TestConnectionError::CannotReach { reason: msg },
        _ => TestConnectionError::Other { reason: msg },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 3 + 4. add_connection / list_connections — share the Connection struct.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddConnectionArgs {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub db: u8,
    pub username: Option<String>,
    pub password: Option<String>,
    pub tls: bool,
    /// Optional CA PEM (additive to system trust roots). `None` /
    /// `Some("")` / whitespace-only → no custom CA. Validated before write.
    pub tls_custom_ca: Option<String>,
    /// Optional SSH tunnel structural config. `None` = no tunnel.
    /// Validated via `core::ssh_config::validate_ssh_tunnel_config` before
    /// the JSON is written to `connections.ssh_tunnel_config`.
    pub ssh_tunnel: Option<SshTunnelConfig>,
    /// SSH password. Only meaningful when
    /// `ssh_tunnel.auth.kind == "password"`. `None` / `Some("")` → no
    /// password stored. `Some(value)` → stored at `{id}::ssh` in the keychain.
    pub ssh_password: Option<String>,
    /// The group this connection belongs to (existing id, or a
    /// name to find-or-create). Chosen explicitly by the user.
    pub group: GroupSelector,
    /// The environment within that group (existing id, or a name
    /// and color to find-or-create). The color is an attribute of the
    /// environment; it is never used to pick one.
    pub environment: EnvironmentSelector,
    /// "Read-only until I unlock it". `None` = the production
    /// default (locked in a Production environment, open elsewhere).
    #[serde(default)]
    pub locked: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Type, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub environment_id: String,
    pub group_id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub db: u8,
    pub username: Option<String>,
    pub tls: bool,
    /// Persisted custom CA PEM (optional). `None` = system trust
    /// roots only. Public certificate, NOT a secret — lives in SQLite, never
    /// in the keychain.
    pub tls_custom_ca: Option<String>,
    /// Structural SSH tunnel config (decoded from the JSON
    /// `ssh_tunnel_config` column). `None` = no tunnel configured. Does NOT
    /// carry the SSH password — that lives in the keychain at `{id}::ssh`.
    /// On a corrupt JSON in the column, the read path warns and returns
    /// `None` rather than failing the whole list query.
    pub ssh_tunnel: Option<SshTunnelConfig>,
    pub env_color: String,
    pub env_name: String,
    pub env_label: String,
    pub group_name: String,
    pub created_at: String,
    /// MCP read-only override: `Some(true)` = agents can
    /// only read, `Some(false)` = agents may write (subject to approval),
    /// `None` = follow the global default. Visible to agents too, so they
    /// can tell up-front that a write would be refused.
    pub mcp_readonly: Option<bool>,
    /// The connection is folded in the sidebar (its queues
    /// hidden). UI state, persisted so it survives a restart.
    pub sidebar_collapsed: bool,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn add_connection(
    args: AddConnectionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Connection, RanchErrorDto> {
    add_connection_inner(args, state.db.pool(), &state.keychain)
        .await
        .map_err(Into::into)
}

pub async fn add_connection_inner(
    args: AddConnectionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
) -> Result<Connection, RanchError> {
    // Validate CA PEM up-front. Empty/whitespace → None.
    let validated_ca = validate_ca_pem(args.tls_custom_ca.as_deref())?;

    // Validate the SSH config + encode to JSON for storage.
    let ssh_tunnel_json = if let Some(cfg) = &args.ssh_tunnel {
        validate_ssh_tunnel_config(cfg)?;
        Some(encode_to_json(cfg)?)
    } else {
        None
    };

    let now = chrono::Utc::now().to_rfc3339();
    let connection_id = uuid::Uuid::now_v7().to_string();

    // Begin transaction so a keychain failure leaves SQLite untouched.
    let mut tx = pool.begin().await?;

    // The user picks group + environment; anything `New` is
    // created inside this transaction so a later failure rolls it back too.
    let placed = resolve_hierarchy(&mut tx, &args.group, &args.environment, &now).await?;
    let env_id = placed.environment_id.clone();

    // production-color connections default to locked at
    // creation. The migration 0001 already declares the column
    // with `DEFAULT 0`; we bind the computed value explicitly so the
    // intent is on-screen at the INSERT site rather than buried in the
    // schema.
    let is_locked: i64 = match args.locked {
        Some(locked) => i64::from(locked),
        None => i64::from(placed.is_production()),
    };
    // And read-only for agents. Elsewhere the connection
    // follows the global default (NULL).
    let mcp_readonly: Option<i64> = placed.is_production().then_some(1);

    sqlx::query(
        r#"
        INSERT INTO connections (
            id, environment_id, name, redis_host, redis_port, redis_db,
            redis_username, tls_enabled, tls_custom_ca, ssh_tunnel_config,
            keychain_ref, is_locked, mcp_readonly, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&connection_id)
    .bind(&env_id)
    .bind(&args.name)
    .bind(&args.host)
    .bind(args.port as i64)
    .bind(args.db as i64)
    .bind(&args.username)
    .bind(if args.tls { 1_i64 } else { 0_i64 })
    .bind(&validated_ca)
    .bind(&ssh_tunnel_json)
    .bind(&connection_id) // keychain_ref convention: same as connection id
    .bind(is_locked)
    .bind(mcp_readonly)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await?;

    // Store credential AFTER the row insert is queued but BEFORE commit.
    // If keychain.set fails, the dropped tx auto-rolls back the insert —
    // we never end up with a connection row that points at a missing key.
    // The `?` propagates the error which causes `tx` to drop without commit,
    // triggering the automatic SQLite rollback.
    if let Some(password) = &args.password {
        keychain.set(&connection_id, password)?;
    }

    // SSH password (separate keychain entry at `{id}::ssh`).
    // Only stored when (a) SSH tunnel is enabled AND (b) auth method is
    // password-based AND (c) a non-empty password is supplied. Otherwise
    // the entry is omitted (key-file or agent auth — the runtime
    // resolves the credential at use time).
    if let (Some(cfg), Some(secret)) = (&args.ssh_tunnel, &args.ssh_password) {
        if matches!(cfg.auth, ssh_config::SshAuthMethod::Password) && !secret.is_empty() {
            keychain.set(&ssh_keychain_name(&connection_id), secret)?;
        }
    }

    tx.commit().await?;

    Ok(Connection {
        id: connection_id,
        environment_id: env_id,
        group_id: placed.group_id,
        name: args.name,
        host: args.host,
        port: args.port,
        db: args.db,
        username: args.username,
        tls: args.tls,
        tls_custom_ca: validated_ca,
        ssh_tunnel: args.ssh_tunnel,
        env_color: placed.env_color,
        env_name: placed.env_name,
        env_label: placed.env_label,
        group_name: placed.group_name,
        created_at: now,
        mcp_readonly: mcp_readonly.map(|v| v != 0),
        sidebar_collapsed: false,
    })
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_connections(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Connection>, RanchErrorDto> {
    list_connections_inner(state.db.pool())
        .await
        .map_err(Into::into)
}

/// Row shape shared by `list_connections_inner` and
/// `fetch_connection_by_id_inner`. A struct rather than a tuple because
/// sqlx only implements `FromRow` for tuples of up to 16 columns.
#[derive(sqlx::FromRow)]
struct ConnectionRow {
    id: String,
    environment_id: String,
    name: String,
    redis_host: String,
    redis_port: i64,
    redis_db: i64,
    redis_username: Option<String>,
    tls_enabled: i64,
    tls_custom_ca: Option<String>,
    ssh_tunnel_config: Option<String>, // JSON-encoded
    created_at: String,
    env_color: String,
    env_label: String,
    env_name: String,
    group_id: String,
    group_name: String,
    mcp_readonly: Option<i64>,
    sidebar_collapsed: i64,
}

/// SELECT + JOINs producing a `ConnectionRow`; callers append WHERE / ORDER BY.
const CONNECTION_SELECT: &str = r#"
    SELECT c.id, c.environment_id, c.name, c.redis_host, c.redis_port, c.redis_db,
           c.redis_username, c.tls_enabled, c.tls_custom_ca, c.ssh_tunnel_config,
           c.created_at,
           e.color_token AS env_color, e.label AS env_label, e.name AS env_name,
           g.id AS group_id, g.name AS group_name,
           c.mcp_readonly, c.sidebar_collapsed
    FROM connections c
    JOIN environments e ON c.environment_id = e.id
    JOIN groups g ON e.group_id = g.id
"#;

impl ConnectionRow {
    fn into_connection(self) -> Connection {
        let ssh_tunnel = decode_ssh_tunnel_column(&self.ssh_tunnel_config, &self.id);
        Connection {
            id: self.id,
            environment_id: self.environment_id,
            group_id: self.group_id,
            name: self.name,
            host: self.redis_host,
            port: self.redis_port as u16,
            db: self.redis_db as u8,
            username: self.redis_username,
            tls: self.tls_enabled != 0,
            tls_custom_ca: self.tls_custom_ca,
            ssh_tunnel,
            env_color: self.env_color,
            env_name: self.env_name,
            env_label: self.env_label,
            group_name: self.group_name,
            created_at: self.created_at,
            mcp_readonly: self.mcp_readonly.map(|v| v != 0),
            sidebar_collapsed: self.sidebar_collapsed != 0,
        }
    }
}

/// Decode the JSON `ssh_tunnel_config` column. Defensive: on a
/// corrupt/unparseable JSON, log a warning and return None so the sidebar
/// keeps working. The user can re-edit the connection to fix it.
fn decode_ssh_tunnel_column(raw: &Option<String>, connection_id: &str) -> Option<SshTunnelConfig> {
    let s = raw.as_deref()?;
    if s.is_empty() {
        return None;
    }
    match decode_from_json(s) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            tracing::warn!(
                connection_id = %connection_id,
                error = %e,
                "ssh_tunnel_config JSON decode failed; falling back to None"
            );
            None
        }
    }
}

pub async fn list_connections_inner(pool: &SqlitePool) -> Result<Vec<Connection>, RanchError> {
    // Flat list in sidebar order; the frontend shapes the tree (`treeShape.ts`).
    let sql =
        format!("{CONNECTION_SELECT} ORDER BY g.display_order, e.display_order, c.created_at");
    let rows: Vec<ConnectionRow> = sqlx::query_as(&sql).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(ConnectionRow::into_connection)
        .collect())
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. ping_connection
// ─────────────────────────────────────────────────────────────────────────────

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

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn ping_connection(
    connection_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<PingConnectionResult, RanchErrorDto> {
    ping_connection_inner(
        &connection_id,
        state.db.pool(),
        &state.keychain,
        Some(state.connections.as_ref()),
    )
    .await
    .map_err(Into::into)
}

/// Row tuple for the `ping_connection_inner` query — aliased so clippy's
/// `type_complexity` lint stays quiet (the columns include
/// ssh_tunnel_config, `is_locked`, `name`, and the joined `env.color_token` so
/// the connection actor can spawn with full identity + initial lock state).
type PingConnectionRow = (
    String,         // redis_host
    i64,            // redis_port
    i64,            // redis_db
    Option<String>, // redis_username
    i64,            // tls_enabled
    Option<String>, // tls_custom_ca
    Option<String>, // ssh_tunnel_config (JSON)
    String,         // keychain_ref
    i64,            // is_locked
    String,         // name
    Option<String>, // env_color (joined from environments.color_token)
);

/// Shared helper that resolves a `ConnectionConfig`
/// from (SQLite row + keychain credential). Used by every command that
/// dispatches to the connection actor (`ping_connection`, `list_queues`,
/// future `list_jobs` / destructive ops).
///
/// Decoder responsibilities:
///   - SQL: `connections` join, returning the 8-tuple defined by
///     `PingConnectionRow`.
///   - SSH tunnel JSON decode (defensive — corrupt JSON → None).
///   - Keychain credential fetch (host -> password resolution).
pub async fn resolve_connection_config(
    connection_id: &str,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
) -> Result<ConnectionConfig, RanchError> {
    // Join `environments` to pull `color_token` so the
    // actor's `env_color` snapshot is filled for audit-log row
    // stripes. `LEFT JOIN` so a connection with a missing env row
    // (shouldn't happen — FK constraint — but defensive) still
    // resolves with `env_color = None`.
    let row: Option<PingConnectionRow> = sqlx::query_as(
        r#"
            SELECT c.redis_host, c.redis_port, c.redis_db, c.redis_username,
                   c.tls_enabled, c.tls_custom_ca, c.ssh_tunnel_config, c.keychain_ref,
                   c.is_locked, c.name, e.color_token AS env_color
            FROM connections c
            LEFT JOIN environments e ON c.environment_id = e.id
            WHERE c.id = ?
            "#,
    )
    .bind(connection_id)
    .fetch_optional(pool)
    .await?;

    let (
        host,
        port,
        db,
        username,
        tls_enabled,
        tls_custom_ca,
        ssh_tunnel_raw,
        keychain_ref,
        is_locked,
        name,
        env_color,
    ) = row.ok_or_else(|| RanchError::ConnectionNotFound {
        id: connection_id.to_string(),
    })?;

    let password = keychain.get(&keychain_ref)?;
    let ssh_tunnel = decode_ssh_tunnel_column(&ssh_tunnel_raw, connection_id);

    // Resolve the SSH password too when the tunnel uses
    // password auth. Stored at `{id}::ssh`.
    let ssh_password = match ssh_tunnel.as_ref().map(|c| &c.auth) {
        Some(SshAuthMethod::Password) => keychain.get(&ssh_keychain_name(connection_id))?,
        _ => None,
    };

    let initial_lock_state = if is_locked != 0 {
        ReadOnlyState::Locked
    } else {
        ReadOnlyState::Unlocked
    };

    Ok(ConnectionConfig {
        host,
        port: port as u16,
        db: db as u8,
        username,
        password,
        tls: tls_enabled != 0,
        tls_custom_ca,
        ssh_tunnel,
        initial_lock_state,
        connection_id: connection_id.to_string(),
        connection_name: name,
        env_color,
        // Clone the pool handle so the connection actor
        // can write audit entries from its destructive handlers
        // (retry / remove / promote). Cheap: SqlitePool is Arc-wrapped.
        pool: pool.clone(),
        // SSH password resolved above + keychain Arc for
        // the actor's hostkey-pin checks.
        ssh_password,
        keychain: Arc::clone(keychain),
    })
}

pub async fn ping_connection_inner(
    connection_id: &str,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: Option<&ConnectionRegistry>,
) -> Result<PingConnectionResult, RanchError> {
    // 1+2. Resolve config (SQL + keychain) via the shared helper.
    let config = resolve_connection_config(connection_id, pool, keychain).await?;

    // 3. Route through the actor registry when available.
    // The actor handles SSH-placeholder short-circuit, TLS CA validation,
    // 2-second timeout, and structured error mapping internally.
    //
    // Tests that don't construct a registry pass `None` and fall back to
    // the legacy direct-Redis path via `ping_with_args`. This preserves the
    // older integration tests without forcing them to spin up a registry
    // just to exercise SQLite + keychain wiring.
    if let Some(registry) = registry {
        return registry
            .ping(connection_id, &config)
            .await
            .map_err(|e| RanchError::InvalidInput {
                reason: format!("Connection actor error: {e}"),
            });
    }

    // Fallback: direct path (test-only). Mirrors the behavior without a registry.
    let args = TestConnectionArgs {
        host: config.host,
        port: config.port,
        db: config.db,
        username: config.username,
        password: config.password,
        tls: config.tls,
        tls_custom_ca: config.tls_custom_ca,
        ssh_tunnel: config.ssh_tunnel,
        ssh_password: config.ssh_password,
        saved_connection_id: None,
    };
    Ok(ping_with_args_with_keychain(&args, connection_id, Arc::clone(keychain)).await)
}

/// Shared between `test_connection_inner` (called from the wizard form) and
/// `ping_connection_inner` (called from the sidebar reachability indicator).
/// Wraps a one-shot PING in a 2 s timeout and returns the structured outcome.
///
/// This helper avoids duplicating the timeout + error-mapping plumbing. The two
/// callers differ only in: (a) where the args come from (form values vs.
/// DB+keychain), (b) whether the result includes the Redis version
/// (test_connection does, ping_connection doesn't — sidebar has no need for it).
#[allow(dead_code)]
pub(crate) async fn ping_with_args(args: &TestConnectionArgs) -> PingConnectionResult {
    // Most callers (tests, simple direct pings) don't need the
    // SSH-tunnel runtime. Delegate to the keychain-aware variant with
    // an in-memory keychain placeholder; the SSH branch only fires
    // when args.ssh_tunnel is Some.
    let synthetic_id = format!("ping::{}:{}", args.host, args.port);
    let keychain: Arc<dyn KeychainAdapter> =
        Arc::new(crate::core::keychain::InMemoryKeychain::new());
    ping_with_args_with_keychain(args, &synthetic_id, keychain).await
}

/// keychain-aware variant of `ping_with_args` that can
/// open an SSH tunnel for the call. The keychain is needed for the
/// TOFU host-key pin (see `core::ssh_tunnel`).
pub(crate) async fn ping_with_args_with_keychain(
    args: &TestConnectionArgs,
    connection_id: &str,
    keychain: Arc<dyn KeychainAdapter>,
) -> PingConnectionResult {
    // If SSH is configured, open the tunnel one-shot for
    // this ping, then dispose. The sidebar's reachability path uses
    // the actor registry which caches; this fallback fires only in
    // tests that pass `registry: None`.
    if let Some(ssh_cfg) = args.ssh_tunnel.as_ref() {
        let start = std::time::Instant::now();
        let open_result = crate::core::ssh_tunnel::open(
            ssh_cfg,
            args.ssh_password.as_deref(),
            (&args.host, args.port),
            keychain,
            connection_id,
        )
        .await;
        match open_result {
            Err(e) => {
                let err = ssh_ranch_err_to_test_err(&e).unwrap_or(TestConnectionError::Other {
                    reason: e.to_string(),
                });
                return PingConnectionResult {
                    success: false,
                    latency_ms: None,
                    error: Some(err),
                };
            }
            Ok(opened) => {
                let local = opened.tunnel.local_addr();
                let url = build_redis_url_for_local_loopback(args, local);
                let r = tokio::time::timeout(
                    std::time::Duration::from_millis(20_000),
                    ping_only(&url, None),
                )
                .await;
                let latency_ms = start.elapsed().as_millis() as u32;
                drop(opened.tunnel);
                return match r {
                    Ok(Ok(())) => PingConnectionResult {
                        success: true,
                        latency_ms: Some(latency_ms),
                        error: None,
                    },
                    Ok(Err(e)) => PingConnectionResult {
                        success: false,
                        latency_ms: None,
                        error: Some(map_redis_err(e)),
                    },
                    Err(_elapsed) => PingConnectionResult {
                        success: false,
                        latency_ms: None,
                        error: Some(TestConnectionError::Timeout),
                    },
                };
            }
        }
    }

    // Validate the CA PEM. If invalid, surface as a typed TLS-shaped error
    // in the result (the outer Result<_, RanchError> is reserved for system
    // errors; user-typed-bad-PEM is a value-level failure for the caller).
    let validated_ca = match validate_ca_pem(args.tls_custom_ca.as_deref()) {
        Ok(v) => v,
        Err(e) => {
            return PingConnectionResult {
                success: false,
                latency_ms: None,
                error: Some(TestConnectionError::TlsHandshake {
                    reason: e.to_string(),
                }),
            };
        }
    };
    let url = build_redis_url(args);
    let start = std::time::Instant::now();
    let result = tokio::time::timeout(
        // Sized for a slow link, was 2 s.
        std::time::Duration::from_millis(15_000),
        ping_only(&url, validated_ca.as_deref()),
    )
    .await;
    let latency_ms = start.elapsed().as_millis() as u32;

    match result {
        Ok(Ok(())) => PingConnectionResult {
            success: true,
            latency_ms: Some(latency_ms),
            error: None,
        },
        Ok(Err(e)) => PingConnectionResult {
            success: false,
            latency_ms: None,
            error: Some(map_redis_err(e)),
        },
        Err(_elapsed) => PingConnectionResult {
            success: false,
            latency_ms: None,
            error: Some(TestConnectionError::Timeout),
        },
    }
}

async fn ping_only(url: &str, tls_custom_ca: Option<&str>) -> Result<(), redis::RedisError> {
    let client = build_redis_client(url, tls_custom_ca)?;
    let mut conn = client
        .get_multiplexed_async_connection_with_config(&crate::core::tls::async_connection_config())
        .await?;
    let _: String = redis::cmd("PING").query_async(&mut conn).await?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 5b. list_environments (MCP-driven)
// ─────────────────────────────────────────────────────────────────────────────

/// agent-readable shape of an environment row. The UI today
/// derives the env list from `list_connections` join data; MCP needs a
/// direct fetch so an agent can enumerate environments without listing
/// every connection.
#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub id: String,
    pub group_id: String,
    /// Lets an agent (and the "Move to…" menu) say
    /// "Lumen / Production" without a second lookup.
    pub group_name: String,
    pub label: String,
    pub color_token: String,
    pub name: String,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_environments(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Environment>, RanchErrorDto> {
    list_environments_inner(state.db.pool())
        .await
        .map_err(Into::into)
}

pub async fn list_environments_inner(pool: &SqlitePool) -> Result<Vec<Environment>, RanchError> {
    // Sidebar order: group, then environment (was alphabetical by label).
    let rows: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
        r#"
        SELECT e.id, e.group_id, g.name, e.label, e.color_token, e.name
        FROM environments e
        JOIN groups g ON e.group_id = g.id
        ORDER BY g.display_order, g.created_at, e.display_order, e.created_at
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, group_id, group_name, label, color_token, name)| Environment {
                id,
                group_id,
                group_name,
                label,
                color_token,
                name,
            },
        )
        .collect())
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. rename_environment
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RenameEnvironmentArgs {
    pub environment_id: String,
    pub new_label: String,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn rename_environment(
    args: RenameEnvironmentArgs,
    state: tauri::State<'_, AppState>,
) -> Result<(), RanchErrorDto> {
    rename_environment_inner(&args.environment_id, &args.new_label, state.db.pool())
        .await
        .map_err(Into::into)
}

pub async fn rename_environment_inner(
    environment_id: &str,
    new_label: &str,
    pool: &SqlitePool,
) -> Result<(), RanchError> {
    let trimmed = new_label.trim();
    if trimmed.is_empty() {
        return Err(RanchError::InvalidInput {
            reason: "Environment label cannot be empty".to_string(),
        });
    }

    let now = chrono::Utc::now().to_rfc3339();
    let result = sqlx::query("UPDATE environments SET label = ?, updated_at = ? WHERE id = ?")
        .bind(trimmed)
        .bind(&now)
        .bind(environment_id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(RanchError::EnvironmentNotFound {
            id: environment_id.to_string(),
        });
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 7. fetch_connection_by_id — shared helper
// ─────────────────────────────────────────────────────────────────────────────

/// Fetch a single connection by id, returning the same joined `Connection` shape
/// `list_connections_inner` produces. Returns `Ok(None)` if no row matches.
///
/// This helper exists because `update_connection` and `duplicate_connection`
/// both need to return the post-write view of the connection, including the
/// joined env/group fields.
pub async fn fetch_connection_by_id_inner(
    id: &str,
    pool: &SqlitePool,
) -> Result<Option<Connection>, RanchError> {
    let sql = format!("{CONNECTION_SELECT} WHERE c.id = ?");
    let row: Option<ConnectionRow> = sqlx::query_as(&sql).bind(id).fetch_optional(pool).await?;
    Ok(row.map(ConnectionRow::into_connection))
}

// ─────────────────────────────────────────────────────────────────────────────
// 8. update_connection
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateConnectionArgs {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub db: u8,
    pub username: Option<String>,
    /// `None` = leave the keychain entry untouched.
    /// `Some("")` = rejected as `InvalidInput` (the `Option<String>` is the
    /// "unchanged" sentinel, an empty string is presumed to be a UI bug).
    /// `Some(value)` = overwrite the keychain entry under the same connection id.
    pub password: Option<String>,
    pub tls: bool,
    /// TLS custom CA semantics differ from `password`:
    ///   - `None` → leave the column untouched.
    ///   - `Some("")` / whitespace-only → CLEAR the custom CA (write NULL,
    ///     fall back to system roots). Unlike `password`, empty here is the
    ///     active "remove" gesture, NOT a UI bug.
    ///   - `Some(pem)` → validate + write.
    pub tls_custom_ca: Option<String>,
    /// SSH tunnel structural config update. Three-way semantics
    /// (combine with `ssh_tunnel_clear` below):
    ///   - `clear=false`, `Some(cfg)` → validate + write JSON to column.
    ///   - `clear=false`, `None` → leave column untouched.
    ///   - `clear=true` (regardless of `ssh_tunnel`) → set column to NULL
    ///     AND delete `{id}::ssh` keychain entry.
    pub ssh_tunnel: Option<SshTunnelConfig>,
    /// Explicit "remove SSH tunnel" gesture. See `ssh_tunnel`.
    pub ssh_tunnel_clear: bool,
    /// SSH password (Option<String> with the same semantics as
    /// `password`):
    ///   - `None` → leave keychain entry untouched.
    ///   - `Some("")` → REJECT as `InvalidInput` (presumed UI bug).
    ///   - `Some(value)` → overwrite at `{id}::ssh`.
    ///
    /// Combined with `ssh_tunnel_clear`: when clear is set, this field is
    /// ignored (the entry is being deleted regardless).
    pub ssh_password: Option<String>,
    /// Where the connection lives. Sending a different group /
    /// environment than the current one **moves** the connection.
    pub group: GroupSelector,
    pub environment: EnvironmentSelector,
    /// "Read-only until I unlock it": `Some` sets the lock the
    /// connection starts with; `None` leaves it (moving into Production
    /// still locks).
    #[serde(default)]
    pub locked: Option<bool>,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn update_connection(
    args: UpdateConnectionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Connection, RanchErrorDto> {
    let id = args.id.clone();
    let updated = update_connection_inner(args, state.db.pool(), &state.keychain)
        .await
        .map_err(RanchErrorDto::from)?;
    // The actor resolves host / credentials / lock state once, at spawn.
    // Drop it so the next command respawns it with what was just saved —
    // otherwise an edit (or a move into Production) only took effect after
    // an app restart. Dropping also ends any session unlock: the safe side.
    state.connections.shutdown(&id).await;
    Ok(updated)
}

pub async fn update_connection_inner(
    args: UpdateConnectionArgs,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
) -> Result<Connection, RanchError> {
    if matches!(&args.password, Some(s) if s.is_empty()) {
        return Err(RanchError::InvalidInput {
            reason: "Password cannot be empty; pass null to leave unchanged".to_string(),
        });
    }

    // SSH password semantics mirror `password` exactly.
    if matches!(&args.ssh_password, Some(s) if s.is_empty()) {
        return Err(RanchError::InvalidInput {
            reason: "SSH password cannot be empty; pass null to leave unchanged".to_string(),
        });
    }

    // When `Some(_)` is provided, validate. Empty/whitespace
    // normalizes to `Some(None)` meaning "clear the column". `None` outer
    // means "leave the column untouched".
    let new_tls_ca: Option<Option<String>> = match args.tls_custom_ca.as_deref() {
        None => None,
        Some(raw) => Some(validate_ca_pem(Some(raw))?),
    };

    // pre-validate the new SSH config (if any) before opening tx.
    // The `clear` flag short-circuits any provided `ssh_tunnel` value.
    let new_ssh_json: Option<Option<String>> = if args.ssh_tunnel_clear {
        Some(None) // explicit clear → write NULL
    } else if let Some(cfg) = &args.ssh_tunnel {
        validate_ssh_tunnel_config(cfg)?;
        Some(Some(encode_to_json(cfg)?))
    } else {
        None // leave column untouched
    };

    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;

    // Existence check first so an unknown id reports ConnectionNotFound
    // rather than a hierarchy error.
    let current: Option<(String,)> =
        sqlx::query_as("SELECT environment_id FROM connections WHERE id = ?")
            .bind(&args.id)
            .fetch_optional(&mut *tx)
            .await?;
    let (current_env_id,) = current.ok_or_else(|| RanchError::ConnectionNotFound {
        id: args.id.clone(),
    })?;

    // Explicit placement. A different environment = a move.
    let placed = resolve_hierarchy(&mut tx, &args.group, &args.environment, &now).await?;
    let target_env_id = placed.environment_id.clone();
    let moved_into_production = target_env_id != current_env_id && placed.is_production();

    // Update non-TLS-CA columns first.
    sqlx::query(
        r#"
        UPDATE connections SET
            name = ?, redis_host = ?, redis_port = ?, redis_db = ?,
            redis_username = ?, tls_enabled = ?, environment_id = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(&args.name)
    .bind(&args.host)
    .bind(args.port as i64)
    .bind(args.db as i64)
    .bind(&args.username)
    .bind(if args.tls { 1_i64 } else { 0_i64 })
    .bind(&target_env_id)
    .bind(&now)
    .bind(&args.id)
    .execute(&mut *tx)
    .await?;

    // Moving a connection INTO a Production-colored environment
    // applies the production default (locked). Moving out never unlocks.
    if moved_into_production {
        sqlx::query("UPDATE connections SET is_locked = 1, mcp_readonly = 1 WHERE id = ?")
            .bind(&args.id)
            .execute(&mut *tx)
            .await?;
    }
    // An explicit choice wins.
    if let Some(locked) = args.locked {
        sqlx::query("UPDATE connections SET is_locked = ? WHERE id = ?")
            .bind(i64::from(locked))
            .bind(&args.id)
            .execute(&mut *tx)
            .await?;
    }

    // Conditional CA update. The outer `Option` distinguishes
    // "leave alone" (None) from "set this value, possibly NULL" (Some(_)).
    if let Some(new_value) = new_tls_ca {
        sqlx::query("UPDATE connections SET tls_custom_ca = ?, updated_at = ? WHERE id = ?")
            .bind(&new_value)
            .bind(&now)
            .bind(&args.id)
            .execute(&mut *tx)
            .await?;
    }

    // Conditional SSH tunnel column update. Same Option-of-Option
    // pattern as the CA: `None` outer = leave alone; `Some(_)` outer = write
    // (potentially NULL).
    if let Some(new_json) = &new_ssh_json {
        sqlx::query("UPDATE connections SET ssh_tunnel_config = ?, updated_at = ? WHERE id = ?")
            .bind(new_json)
            .bind(&now)
            .bind(&args.id)
            .execute(&mut *tx)
            .await?;
    }

    // Conditional keychain update — None means leave alone. Failure aborts
    // the tx (same rollback semantics as add_connection).
    if let Some(pw) = &args.password {
        keychain.set(&args.id, pw)?;
    }

    // SSH password update / cleanup logic.
    //
    // Three independent gestures, in priority order:
    //   1. `ssh_tunnel_clear == true` → delete the keychain entry (cleanup
    //      regardless of `ssh_password` — clear means "remove SSH entirely").
    //   2. New SSH config switched to KeyFile auth → delete any existing
    //      password entry (orphan cleanup).
    //   3. `ssh_password.is_some()` → set the entry (rotation / new password).
    let ssh_kc = ssh_keychain_name(&args.id);
    if args.ssh_tunnel_clear {
        // Ignore NoEntry; warn but proceed on KeychainUnavailable (matches
        // remove_connection's pattern).
        if let Err(e) = keychain.delete(&ssh_kc) {
            if let RanchError::KeychainUnavailable { .. } = &e {
                tracing::warn!(error = %e, connection_id = %args.id, "keychain unavailable while clearing SSH password; proceeding");
            } else {
                return Err(e);
            }
        }
    } else if let Some(cfg) = &args.ssh_tunnel {
        if matches!(cfg.auth, ssh_config::SshAuthMethod::KeyFile { .. }) {
            // Auth switched to key-file → drop any orphan password.
            if let Err(e) = keychain.delete(&ssh_kc) {
                if let RanchError::KeychainUnavailable { .. } = &e {
                    tracing::warn!(error = %e, connection_id = %args.id, "keychain unavailable during key-file orphan cleanup; proceeding");
                } else {
                    return Err(e);
                }
            }
        }
    }
    if !args.ssh_tunnel_clear {
        if let Some(pw) = &args.ssh_password {
            keychain.set(&ssh_kc, pw)?;
        }
    }

    tx.commit().await?;

    fetch_connection_by_id_inner(&args.id, pool)
        .await?
        .ok_or(RanchError::ConnectionNotFound { id: args.id })
}

// ─────────────────────────────────────────────────────────────────────────────
// 9. duplicate_connection
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateConnectionArgs {
    pub id: String,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn duplicate_connection(
    args: DuplicateConnectionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Connection, RanchErrorDto> {
    duplicate_connection_inner(&args.id, state.db.pool(), &state.keychain)
        .await
        .map_err(Into::into)
}

pub async fn duplicate_connection_inner(
    source_id: &str,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
) -> Result<Connection, RanchError> {
    let now = chrono::Utc::now().to_rfc3339();
    let new_id = uuid::Uuid::now_v7().to_string();
    let mut tx = pool.begin().await?;

    // Read source row.
    type SourceRow = (
        String,         // environment_id
        String,         // name
        String,         // redis_host
        i64,            // redis_port
        i64,            // redis_db
        Option<String>, // redis_username
        i64,            // tls_enabled
        Option<String>, // tls_custom_ca
        Option<String>, // ssh_tunnel_config
        String,         // keychain_ref
    );
    let source: Option<SourceRow> = sqlx::query_as(
        r#"
        SELECT environment_id, name, redis_host, redis_port, redis_db,
               redis_username, tls_enabled, tls_custom_ca, ssh_tunnel_config,
               keychain_ref
        FROM connections WHERE id = ?
        "#,
    )
    .bind(source_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (
        env_id,
        source_name,
        host,
        port,
        db,
        username,
        tls,
        tls_custom_ca,
        ssh_tunnel_json,
        source_keychain_ref,
    ) = source.ok_or_else(|| RanchError::ConnectionNotFound {
        id: source_id.to_string(),
    })?;

    // Resolve a non-colliding name within the same env.
    let existing_names: Vec<(String,)> =
        sqlx::query_as("SELECT name FROM connections WHERE environment_id = ?")
            .bind(&env_id)
            .fetch_all(&mut *tx)
            .await?;
    let existing_names: Vec<String> = existing_names.into_iter().map(|(n,)| n).collect();
    let new_name = next_duplicate_name(&source_name, &existing_names);

    // Insert new row pointing at the new keychain_ref (same convention: keychain_ref = id).
    // Carry the source's tls_custom_ca to the duplicate.
    // Carry the source's ssh_tunnel_config to the duplicate.
    sqlx::query(
        r#"
        INSERT INTO connections (
            id, environment_id, name, redis_host, redis_port, redis_db,
            redis_username, tls_enabled, tls_custom_ca, ssh_tunnel_config,
            keychain_ref, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&new_id)
    .bind(&env_id)
    .bind(&new_name)
    .bind(&host)
    .bind(port)
    .bind(db)
    .bind(&username)
    .bind(tls)
    .bind(&tls_custom_ca)
    .bind(&ssh_tunnel_json)
    .bind(&new_id)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await?;

    // Copy the credential into a new keychain entry. Skip if source had none.
    if let Some(secret) = keychain.get(&source_keychain_ref)? {
        keychain.set(&new_id, &secret)?;
    }

    // Copy the SSH password keychain entry too (independent of
    // the Redis password). Skip if source had none.
    if let Some(secret) = keychain.get(&ssh_keychain_name(source_id))? {
        keychain.set(&ssh_keychain_name(&new_id), &secret)?;
    }

    tx.commit().await?;

    fetch_connection_by_id_inner(&new_id, pool)
        .await?
        .ok_or(RanchError::ConnectionNotFound { id: new_id })
}

/// Compute a non-colliding name for a duplicated connection. Strips an
/// existing " (copy)" / " (copy N)" suffix to avoid `"foo (copy) (copy)"`,
/// then appends " (copy)", " (copy 2)", " (copy 3)", ... as needed.
pub(crate) fn next_duplicate_name(source: &str, existing_names: &[String]) -> String {
    let base = strip_copy_suffix(source);
    let first = format!("{base} (copy)");
    if !existing_names.iter().any(|x| x == &first) {
        return first;
    }
    let mut n = 2_u32;
    loop {
        let candidate = format!("{base} (copy {n})");
        if !existing_names.iter().any(|x| x == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn strip_copy_suffix(name: &str) -> &str {
    if let Some(stripped) = name.strip_suffix(" (copy)") {
        return stripped;
    }
    // " (copy 2)" / " (copy 3)" / ...
    if let Some(idx) = name.rfind(" (copy ") {
        let tail = &name[idx + " (copy ".len()..];
        if let Some(num_end) = tail.strip_suffix(')') {
            if num_end.parse::<u32>().is_ok() {
                return &name[..idx];
            }
        }
    }
    name
}

// ─────────────────────────────────────────────────────────────────────────────
// 10. remove_connection
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn remove_connection(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), RanchErrorDto> {
    remove_connection_inner(
        &id,
        state.db.pool(),
        &state.keychain,
        Some(state.connections.as_ref()),
    )
    .await
    .map_err(Into::into)
}

pub async fn remove_connection_inner(
    id: &str,
    pool: &SqlitePool,
    keychain: &Arc<dyn KeychainAdapter>,
    registry: Option<&ConnectionRegistry>,
) -> Result<(), RanchError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT keychain_ref FROM connections WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    let (keychain_ref,) =
        row.ok_or_else(|| RanchError::ConnectionNotFound { id: id.to_string() })?;

    // Delete keychain entries FIRST. Rationale: leaking a stale credential in
    // the keychain is preferable to leaving a row pointing at a missing one
    // (which would surface as KeychainUnavailable on every read).
    //
    // Clean up THREE entries:
    //   1. Redis password at `{id}`.
    //   2. SSH password at `{id}::ssh`.
    //   3. SSH hostkey pin at `{id}::ssh::hostkey`.
    // Each delete is independent: if one fails with KeychainUnavailable we log
    // + continue (matches the `remove_connection` pattern).
    let ssh_kc = ssh_keychain_name(id);
    let ssh_hostkey_kc = ssh_hostkey_keychain_name(id);
    for entry_name in [
        keychain_ref.as_str(),
        ssh_kc.as_str(),
        ssh_hostkey_kc.as_str(),
    ] {
        if let Err(e) = keychain.delete(entry_name) {
            if let RanchError::KeychainUnavailable { .. } = &e {
                tracing::warn!(
                    error = %e,
                    connection_id = %id,
                    entry = %entry_name,
                    "keychain unavailable during remove; proceeding"
                );
            } else {
                return Err(e);
            }
        }
    }

    sqlx::query("DELETE FROM connections WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    // Shut down the live actor (if any) AFTER the SQLite row
    // is gone. Order matters: if the DELETE failed we want the actor to
    // stay alive (caller may retry). Once the row is deleted, the actor
    // is orphaned — drop it cleanly. Tests pass `None` to skip this.
    if let Some(registry) = registry {
        registry.shutdown(id).await;
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_basic_url() {
        let r = parse_redis_url_inner("redis://default:secret@redis.example.com:6379/0".into())
            .await
            .unwrap();
        assert_eq!(r.host, "redis.example.com");
        assert_eq!(r.port, 6379);
        assert_eq!(r.db, 0);
        assert_eq!(r.username.as_deref(), Some("default"));
        assert_eq!(r.password.as_deref(), Some("secret"));
        assert!(!r.tls);
    }

    #[tokio::test]
    async fn parses_rediss_with_db_no_auth() {
        let r = parse_redis_url_inner("rediss://x.upstash.io/3".into())
            .await
            .unwrap();
        assert_eq!(r.host, "x.upstash.io");
        assert_eq!(r.port, 6379);
        assert_eq!(r.db, 3);
        assert!(r.tls);
        assert_eq!(r.username, None);
        assert_eq!(r.password, None);
    }

    #[tokio::test]
    async fn parses_password_only_no_username() {
        let r = parse_redis_url_inner("redis://:onlyPassword@host.local:6380".into())
            .await
            .unwrap();
        assert_eq!(r.host, "host.local");
        assert_eq!(r.port, 6380);
        assert_eq!(r.username, None);
        assert_eq!(r.password.as_deref(), Some("onlyPassword"));
    }

    #[tokio::test]
    async fn parses_explicit_port_and_default_db() {
        let r = parse_redis_url_inner("redis://h:6390".into())
            .await
            .unwrap();
        assert_eq!(r.port, 6390);
        assert_eq!(r.db, 0);
    }

    #[tokio::test]
    async fn rejects_unsupported_scheme() {
        let err = parse_redis_url_inner("http://x".into()).await.unwrap_err();
        match err {
            RanchError::InvalidRedisUrl { reason } => assert!(reason.contains("scheme")),
            other => panic!("expected InvalidRedisUrl, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_malformed_url() {
        let err = parse_redis_url_inner("not a url at all".into())
            .await
            .unwrap_err();
        assert!(matches!(err, RanchError::InvalidRedisUrl { .. }));
    }

    #[tokio::test]
    async fn percent_decoded_password_is_returned_plain() {
        // `%40` = `@`. Real Upstash secrets often contain @ in the password.
        let r = parse_redis_url_inner("redis://default:p%40ss@h:6379".into())
            .await
            .unwrap();
        assert_eq!(r.password.as_deref(), Some("p@ss"));
    }

    #[tokio::test]
    async fn test_connection_returns_cannot_reach_for_dead_port() {
        // 127.0.0.1:1 is reserved/unused — connection refused fast.
        // (An injected keychain: the test runs under both feature sets.)
        let test_connection_inner = |args| {
            test_connection_inner_with_keychain(
                args,
                Arc::new(crate::core::keychain::InMemoryKeychain::new()),
            )
        };
        let r = test_connection_inner(TestConnectionArgs {
            host: "127.0.0.1".into(),
            port: 1,
            db: 0,
            username: None,
            password: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
            ssh_password: None,
            saved_connection_id: None,
        })
        .await
        .unwrap();
        assert!(!r.success);
        assert!(r.error.is_some());
        // Either CannotReach or Timeout depending on platform — both are acceptable
        // signals that the user can't reach the host.
        match r.error.unwrap() {
            TestConnectionError::CannotReach { .. } | TestConnectionError::Timeout => {}
            other => panic!("expected CannotReach or Timeout, got {other:?}"),
        }
    }

    #[test]
    fn build_redis_url_handles_all_auth_combos() {
        let base = TestConnectionArgs {
            host: "h".into(),
            port: 6379,
            db: 0,
            username: None,
            password: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
            ssh_password: None,
            saved_connection_id: None,
        };
        assert_eq!(build_redis_url(&base), "redis://h:6379/0");

        let tls = TestConnectionArgs {
            tls: true,
            ..clone_args(&base)
        };
        assert_eq!(build_redis_url(&tls), "rediss://h:6379/0");

        let pass = TestConnectionArgs {
            password: Some("s".into()),
            ..clone_args(&base)
        };
        assert_eq!(build_redis_url(&pass), "redis://:s@h:6379/0");

        let both = TestConnectionArgs {
            username: Some("u".into()),
            password: Some("s".into()),
            ..clone_args(&base)
        };
        assert_eq!(build_redis_url(&both), "redis://u:s@h:6379/0");
    }

    #[test]
    fn a_blank_password_on_edit_comes_from_the_keychain() {
        let keychain = crate::core::keychain::InMemoryKeychain::new();
        keychain.set("c1", "saved").unwrap();
        keychain.set(&ssh_keychain_name("c1"), "ssh-saved").unwrap();
        let base = TestConnectionArgs {
            host: "h".into(),
            port: 6379,
            db: 0,
            username: None,
            password: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: Some(SshTunnelConfig {
                host: "b".into(),
                port: 22,
                user: "u".into(),
                auth: SshAuthMethod::Password,
                jump_host: None,
            }),
            ssh_password: None,
            saved_connection_id: Some("c1".into()),
        };
        let filled = with_saved_secrets(clone_args(&base), &keychain);
        assert_eq!(filled.password.as_deref(), Some("saved"));
        assert_eq!(filled.ssh_password.as_deref(), Some("ssh-saved"));

        // A typed password wins; a new connection has nothing to fall back on.
        let typed = TestConnectionArgs {
            password: Some("typed".into()),
            ..clone_args(&base)
        };
        assert_eq!(
            with_saved_secrets(typed, &keychain).password.as_deref(),
            Some("typed")
        );
        let fresh = TestConnectionArgs {
            saved_connection_id: None,
            ..clone_args(&base)
        };
        assert_eq!(with_saved_secrets(fresh, &keychain).password, None);
    }

    fn clone_args(a: &TestConnectionArgs) -> TestConnectionArgs {
        TestConnectionArgs {
            host: a.host.clone(),
            port: a.port,
            db: a.db,
            username: a.username.clone(),
            password: a.password.clone(),
            tls: a.tls,
            tls_custom_ca: a.tls_custom_ca.clone(),
            ssh_tunnel: a.ssh_tunnel.clone(),
            ssh_password: a.ssh_password.clone(),
            saved_connection_id: a.saved_connection_id.clone(),
        }
    }

    // ─── duplicate-name helper unit tests ──────────────────────

    #[test]
    fn next_duplicate_name_first_copy() {
        assert_eq!(next_duplicate_name("foo", &[]), "foo (copy)");
    }

    #[test]
    fn next_duplicate_name_second_copy() {
        let existing = vec!["foo".to_string(), "foo (copy)".to_string()];
        assert_eq!(next_duplicate_name("foo", &existing), "foo (copy 2)");
    }

    #[test]
    fn next_duplicate_name_skips_gaps_and_finds_next_free() {
        let existing = vec![
            "foo".to_string(),
            "foo (copy)".to_string(),
            "foo (copy 2)".to_string(),
            "foo (copy 4)".to_string(),
        ];
        assert_eq!(next_duplicate_name("foo", &existing), "foo (copy 3)");
    }

    #[test]
    fn next_duplicate_name_strips_existing_copy_suffix() {
        // Duplicating "foo (copy)" gives "foo (copy 2)", NOT "foo (copy) (copy)".
        let existing = vec!["foo (copy)".to_string()];
        assert_eq!(next_duplicate_name("foo (copy)", &existing), "foo (copy 2)");
    }

    #[test]
    fn next_duplicate_name_strips_numbered_copy_suffix() {
        // Duplicating "foo (copy 3)" treats "foo" as the base.
        let existing = vec!["foo".to_string(), "foo (copy 3)".to_string()];
        assert_eq!(next_duplicate_name("foo (copy 3)", &existing), "foo (copy)");
    }

    #[test]
    fn strip_copy_suffix_passes_through_unrelated_suffixes() {
        assert_eq!(strip_copy_suffix("foo bar"), "foo bar");
        assert_eq!(strip_copy_suffix("foo (copy)"), "foo");
        assert_eq!(strip_copy_suffix("foo (copy 5)"), "foo");
        // Non-numeric suffix is preserved (not a Ranch-generated copy name).
        assert_eq!(strip_copy_suffix("foo (copy x)"), "foo (copy x)");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// set_connection_collapsed
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetConnectionCollapsedArgs {
    pub connection_id: String,
    pub collapsed: bool,
}

/// Fold or unfold a connection in the sidebar.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn set_connection_collapsed(
    args: SetConnectionCollapsedArgs,
    state: tauri::State<'_, AppState>,
) -> Result<(), RanchErrorDto> {
    set_connection_collapsed_inner(&args.connection_id, args.collapsed, state.db.pool())
        .await
        .map_err(Into::into)
}

pub async fn set_connection_collapsed_inner(
    connection_id: &str,
    collapsed: bool,
    pool: &SqlitePool,
) -> Result<(), RanchError> {
    // No `updated_at` bump: folding is not an edit of the connection.
    let result = sqlx::query("UPDATE connections SET sidebar_collapsed = ? WHERE id = ?")
        .bind(if collapsed { 1_i64 } else { 0_i64 })
        .bind(connection_id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(RanchError::ConnectionNotFound {
            id: connection_id.to_string(),
        });
    }
    Ok(())
}
