//! Connection commands.
//!
//! The four commands here form the first end-to-end feature surface in Ranch:
//!
//! 1. [`parse_redis_url`]: pure URL → struct (no IO). Powers the form's
//!    paste-and-fill UX.
//! 2. [`test_connection`]: have the backend of the endpoint's kind probe
//!    it once (`BackendSpec::probe`), and return a structured outcome
//!    (success+version OR a classified error).
//! 3. [`add_connection`]: persist a connection: (optional) keychain entry,
//!    then the SQLite row; the entry is taken back if the row is not saved.
//! 4. [`list_connections`]: read joined connection rows for the sidebar.
//!
//! A connection is a backend kind and that backend's endpoint, a JSON
//! object: the backend of the kind checks it ([`list_backends`] says what
//! each asks for), and the secrets travel beside it, to the keychain.
//!
//! Architecture invariants enforced here:
//! - Credentials never land in SQLite: a connection's password is the
//!   keychain entry named after its id (`ports::secrets::password_entry`).
//! - Commands 3 + 4 split a thin Tauri wrapper from a `_inner` async fn that
//!   takes the `ConnectionRepository` and the keychain, so they're testable
//!   without spinning up a Tauri runtime.

use std::sync::Arc;

pub use crate::contract::connections::{
    BackendInfo, BullmqRedisEndpoint, ConnectionProbe, EndpointField, EndpointSchema, FieldKind,
    JsonValue, PingConnectionResult, SshAuthMethod, SshHostKeyPin, SshJumpHost, SshTunnelConfig,
    TestConnectionError, TestConnectionResult,
};
pub use crate::contract::hierarchy::{Connection, Environment};

use crate::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use crate::adapters::bullmq_redis::ssh_tunnel::ssh_keychain_name;
use crate::app_state::AppState;
use crate::application::ports::connections::{ConnectionEdit, NewConnection};
use crate::application::ports::secrets::password_entry;
use crate::application::ports::SecretStore;
use crate::application::ports::{BackendSpecs, Backends, ConnectionRepository};
use crate::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use crate::contract::errors::RanchErrorDto;
use crate::domain::endpoint::{BackendKind, Endpoint};
use crate::domain::errors::RanchError;
use crate::domain::hierarchy;
use crate::domain::probe::ProbeSecrets;
use crate::domain::ssh_config;
use serde::{Deserialize, Serialize};
use specta::Type;
use url::Url;

/// Keychain entry name for the per-connection SSH
/// host-key pin (TOFU model). Distinct from `{id}` (Redis password)
/// and `{id}::ssh` (SSH password). `remove_connection` cleans it;
/// `duplicate_connection` copies it: the copy reaches the same bastion,
/// and trusting whatever answers its first connect would undo the pin.
pub fn ssh_hostkey_keychain_name(connection_id: &str) -> String {
    format!("{connection_id}::ssh::hostkey")
}

/// The jump host's pin: the tunnel pins it as the connection `{id}::jump`.
pub fn ssh_jump_hostkey_keychain_name(connection_id: &str) -> String {
    ssh_hostkey_keychain_name(&format!("{connection_id}::jump"))
}

/// Keychain entries changed for a save, so a save that fails can put them
/// back. The keychain is never asked while a write transaction is open: a
/// prompt there (macOS asks before a binary reads an item it did not
/// create) would hold every other writer, the audit log included. The
/// repository's writes fail only before their commit (nothing that can
/// fail comes after it), so an error from one always means the row was
/// not saved, and putting the secrets back never leaves a saved row with
/// the old ones.
struct SecretChanges<'a> {
    keychain: &'a dyn SecretStore,
    /// Each entry changed, with what it held before.
    before: Vec<(String, Option<String>)>,
}

impl<'a> SecretChanges<'a> {
    fn new(keychain: &'a dyn SecretStore) -> Self {
        Self {
            keychain,
            before: Vec::new(),
        }
    }

    /// Write an entry nothing names yet (a new connection's).
    fn create(&mut self, entry: String, secret: &str) -> Result<(), RanchError> {
        self.keychain.set(&entry, secret)?;
        self.before.push((entry, None));
        Ok(())
    }

    /// Replace an entry, keeping what it held.
    fn replace(&mut self, entry: String, secret: &str) -> Result<(), RanchError> {
        let previous = self.keychain.get(&entry)?;
        self.keychain.set(&entry, secret)?;
        self.before.push((entry, previous));
        Ok(())
    }

    /// Forget an entry, keeping what it held.
    fn delete(&mut self, entry: String) -> Result<(), RanchError> {
        let previous = self.keychain.get(&entry)?;
        self.keychain.delete(&entry)?;
        self.before.push((entry, previous));
        Ok(())
    }

    /// Put every entry back as it was, the last change first. Best effort.
    fn undo(self) {
        for (entry, previous) in self.before.into_iter().rev() {
            let restored = match &previous {
                Some(secret) => self.keychain.set(&entry, secret),
                None => self.keychain.delete(&entry),
            };
            if let Err(e) = restored {
                tracing::warn!(error = %e, entry = %entry, "could not restore a keychain entry after a failed save");
            }
        }
    }
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

    // /0, /1, ...: default 0 if path is empty or non-numeric.
    let db: u8 = parsed.path().trim_start_matches('/').parse().unwrap_or(0);

    let username = if parsed.username().is_empty() {
        None
    } else {
        Some(parsed.username().to_string())
    };

    // url::Url percent-encodes user/password; decode for display.
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
    /// The kind of backend (`bullmq-redis`): the test reaches a Redis.
    pub backend: String,
    /// The endpoint to test, as the backend describes it. Validated (the CA
    /// PEM, the SSH tunnel) before any connection is opened.
    #[specta(type = JsonValue)]
    pub endpoint: serde_json::Value,
    pub password: Option<String>,
    /// SSH password for the Test call when the endpoint's tunnel uses
    /// password auth. The form passes the current draft value (which is
    /// empty in edit mode unless the user typed a new password, same UX as
    /// the Redis password). `None` falls back to key-file auth or anonymous
    /// attempt.
    #[serde(default)]
    pub ssh_password: Option<String>,
    /// Editing a saved connection: a blank password (Redis or
    /// SSH) means "the one already in the keychain", as it does on save, so
    /// the test does not fail for a password the form never shows.
    #[serde(default)]
    pub saved_connection_id: Option<String>,
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn test_connection(
    args: TestConnectionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<TestConnectionResult, RanchErrorDto> {
    let args = resolve_saved_secrets(
        args,
        state.repository.as_ref(),
        state.backends.specs(),
        state.keychain.as_ref(),
    )
    .await
    .map_err(RanchErrorDto::from)?;
    test_connection_inner(args, state.backends.specs())
        .await
        .map_err(Into::into)
}

/// Credentials go out byte for byte (Redis `AUTH`, SSH). A CR or LF in them
/// is never a real password, and against a line-based service it would add
/// commands of the sender's choosing: refuse every control character.
fn reject_control_chars(fields: &[(&str, Option<&str>)]) -> Result<(), RanchError> {
    for (label, value) in fields {
        if value.is_some_and(|v| v.chars().any(char::is_control)) {
            return Err(RanchError::InvalidInput {
                reason: format!("{label} cannot contain control characters (line breaks, tabs)"),
            });
        }
    }
    Ok(())
}

/// Same bastion, same user, same jump host.
pub(crate) fn same_ssh_endpoint(
    saved: Option<&ssh_config::SshTunnelConfig>,
    new: Option<&ssh_config::SshTunnelConfig>,
) -> bool {
    match (saved, new) {
        (Some(a), Some(b)) => {
            a.host.eq_ignore_ascii_case(b.host.trim())
                && a.port == b.port
                && a.user == b.user
                && a.jump_host == b.jump_host
        }
        _ => false,
    }
}

/// Does the tunnel log in with a password (kept at `{id}::ssh`)?
fn uses_ssh_password(tunnel: Option<&ssh_config::SshTunnelConfig>) -> bool {
    matches!(
        tunnel.map(|t| &t.auth),
        Some(ssh_config::SshAuthMethod::Password)
    )
}

/// Fill a blank password from the keychain of the connection being edited
/// (the entry named after its id), so a test does not fail for a password
/// the form never shows, but **only when the test targets the server the
/// password was saved for**, as the backend of the saved connection judges
/// it. Otherwise the renderer (or, on Ranch server, any signed-in user)
/// could ask for "connection X's password, sent to my host" and read it off
/// the wire. A keychain that cannot be read leaves the password blank: the
/// test then reports what Redis says.
fn with_saved_secrets(
    mut args: TestConnectionArgs,
    saved: Option<&hierarchy::Connection>,
    specs: &BackendSpecs,
    keychain: &dyn SecretStore,
) -> TestConnectionArgs {
    let (Some(id), Some(saved)) = (args.saved_connection_id.clone(), saved) else {
        return args;
    };
    // Another kind of backend is another server.
    let Ok(spec) = specs.get(&saved.backend) else {
        return args;
    };
    if saved.backend.as_str() != args.backend {
        return args;
    }
    let new = Endpoint::new(args.endpoint.clone());
    if args.password.is_none() && spec.same_server(&saved.endpoint, &new) {
        args.password = password_entry(&id)
            .and_then(|entry| keychain.get(entry))
            .unwrap_or(None);
    }
    let tunnel = spec.tunnel(&new);
    if uses_ssh_password(tunnel.as_ref())
        && args.ssh_password.is_none()
        && same_ssh_endpoint(spec.tunnel(&saved.endpoint).as_ref(), tunnel.as_ref())
    {
        args.ssh_password = keychain.get(&ssh_keychain_name(&id)).unwrap_or(None);
    }
    args
}

/// Both shells call this before [`test_connection_inner_with_keychain`]: it
/// looks the saved connection up and fills its secrets where they may go.
pub async fn resolve_saved_secrets(
    args: TestConnectionArgs,
    connections: &dyn ConnectionRepository,
    specs: &BackendSpecs,
    keychain: &dyn SecretStore,
) -> Result<TestConnectionArgs, RanchError> {
    let saved = match args.saved_connection_id.as_deref() {
        Some(id) => connections.get(id).await?,
        None => None,
    };
    Ok(with_saved_secrets(args, saved.as_ref(), specs, keychain))
}

/// Desktop only: it reaches for the OS keychain. The HTTP shell calls
/// [`test_connection_inner_with_keychain`] with its own secret store.
#[cfg(feature = "desktop")]
pub async fn test_connection_inner(
    args: TestConnectionArgs,
    specs: &BackendSpecs,
) -> Result<TestConnectionResult, RanchError> {
    test_connection_inner_with_keychain(args, specs, default_keychain()).await
}

/// Default keychain for Tauri-command callers. Tests inject their own
/// via [`test_connection_inner_with_keychain`].
#[cfg(feature = "desktop")]
fn default_keychain() -> Arc<dyn SecretStore> {
    Arc::new(crate::adapters::secrets::OsKeychain::new())
}

/// Testable variant of `test_connection_inner` that takes
/// an explicit keychain handle. The SSH tunnel layer needs a keychain
/// for the TOFU host-key pin (see `adapters::bullmq_redis::ssh_tunnel`); production
/// callers use the OS keychain, integration tests inject an in-memory
/// one. The backend of the endpoint's kind probes it.
pub async fn test_connection_inner_with_keychain(
    args: TestConnectionArgs,
    specs: &BackendSpecs,
    keychain: Arc<dyn SecretStore>,
) -> Result<TestConnectionResult, RanchError> {
    // Validate the endpoint up-front (the CA PEM, the tunnel, the user
    // name) so a malformed paste surfaces as a typed RanchError::InvalidInput
    // rather than a cryptic mid-handshake failure.
    let spec = specs.get(&BackendKind::new(args.backend))?;
    let endpoint = spec.validate(&Endpoint::new(args.endpoint))?;
    reject_control_chars(&[
        ("The password", args.password.as_deref()),
        ("The SSH password", args.ssh_password.as_deref()),
    ])?;
    let secrets = ProbeSecrets {
        password: args.password,
        ssh_password: args.ssh_password,
    };
    Ok(spec.probe(&endpoint, &secrets, keychain).await?.into())
}

// ─────────────────────────────────────────────────────────────────────────────
// 3 + 4. add_connection / list_connections: share the Connection struct.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddConnectionArgs {
    pub name: String,
    /// The kind of backend that serves it (`bullmq-redis`).
    pub backend: String,
    /// Where that backend is reached, as it describes it. Checked by the
    /// backend of its kind before anything is written. Never a secret.
    #[specta(type = JsonValue)]
    pub endpoint: serde_json::Value,
    /// The connection's secret, stored at `{id}` in the keychain. `None` /
    /// `Some("")` → no password stored.
    pub password: Option<String>,
    /// SSH password. Only meaningful when the endpoint's tunnel uses
    /// password auth. `None` / `Some("")` → no password stored.
    /// `Some(value)` → stored at `{id}::ssh` in the keychain.
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

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn add_connection(
    args: AddConnectionArgs,
    state: tauri::State<'_, AppState>,
) -> Result<Connection, RanchErrorDto> {
    add_connection_inner(
        args,
        state.repository.as_ref(),
        &state.keychain,
        state.backends.specs(),
    )
    .await
    .map_err(Into::into)
}

pub async fn add_connection_inner(
    args: AddConnectionArgs,
    connections: &dyn ConnectionRepository,
    keychain: &Arc<dyn SecretStore>,
    specs: &BackendSpecs,
) -> Result<Connection, RanchError> {
    reject_control_chars(&[
        ("The password", args.password.as_deref()),
        ("The SSH password", args.ssh_password.as_deref()),
    ])?;
    // The backend of the kind checks its endpoint (for BullMQ: the CA, the
    // tunnel, the user name).
    let backend = BackendKind::new(args.backend.clone());
    let spec = specs.get(&backend)?;
    let endpoint = spec.validate(&Endpoint::new(args.endpoint.clone()))?;
    let ssh_tunnel = spec.tunnel(&endpoint);

    let connection_id = uuid::Uuid::now_v7().to_string();

    // The secrets go to the keychain first, under the new id that no row
    // names yet; if the row (and any group or environment created for it)
    // cannot be saved, they are taken back. No connection points at a
    // missing key, and no write transaction waits on the keychain.
    let mut changes = SecretChanges::new(keychain.as_ref());
    let stored = store_new_secrets(
        &mut changes,
        &connection_id,
        args.password.as_deref(),
        args.ssh_password.as_deref(),
        ssh_tunnel.as_ref(),
    );
    if let Err(e) = stored {
        changes.undo();
        return Err(e);
    }

    let saved = connections
        .insert(NewConnection {
            id: connection_id.clone(),
            name: args.name.clone(),
            backend,
            endpoint,
            group: args.group.clone().into(),
            environment: args.environment.clone().into(),
            locked: args.locked,
        })
        .await;
    match saved {
        Ok(connection) => Ok(connection.into()),
        Err(e) => {
            changes.undo();
            Err(e)
        }
    }
}

/// A new connection's secrets. The SSH password (its own entry, at
/// `{id}::ssh`) is stored only when the tunnel logs in with a password and
/// one is given; key-file or agent auth resolves its credential at use time.
fn store_new_secrets(
    changes: &mut SecretChanges<'_>,
    connection_id: &str,
    password: Option<&str>,
    ssh_password: Option<&str>,
    ssh_tunnel: Option<&ssh_config::SshTunnelConfig>,
) -> Result<(), RanchError> {
    if let Some(password) = password {
        changes.create(password_entry(connection_id)?.to_string(), password)?;
    }
    if let Some(secret) = ssh_password {
        if uses_ssh_password(ssh_tunnel) && !secret.is_empty() {
            changes.create(ssh_keychain_name(connection_id), secret)?;
        }
    }
    Ok(())
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_connections(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Connection>, RanchErrorDto> {
    list_connections_inner(state.repository.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn list_connections_inner(
    connections: &dyn ConnectionRepository,
) -> Result<Vec<Connection>, RanchError> {
    let connections = connections.list().await?;
    Ok(connections.into_iter().map(Into::into).collect())
}

// ─────────────────────────────────────────────────────────────────────────────
// 4b. list_backends
// ─────────────────────────────────────────────────────────────────────────────

/// The kinds of backend a connection may name, with what each one's
/// endpoint asks for (the connection form is chosen by the kind) and what
/// it can do (the screens follow it).
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_backends(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<BackendInfo>, RanchErrorDto> {
    Ok(list_backends_inner(state.backends.specs()))
}

pub fn list_backends_inner(specs: &BackendSpecs) -> Vec<BackendInfo> {
    specs
        .iter()
        .map(|spec| BackendInfo {
            kind: spec.kind().as_str().to_string(),
            label: spec.label().to_string(),
            endpoint_schema: spec.endpoint_schema().into(),
            capabilities: spec.capabilities().into(),
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. ping_connection
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn ping_connection(
    connection_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<PingConnectionResult, RanchErrorDto> {
    ping_connection_inner(&connection_id, state.backends.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn ping_connection_inner(
    connection_id: &str,
    backends: &dyn Backends,
) -> Result<PingConnectionResult, RanchError> {
    backends
        .open(connection_id)
        .await?
        .connection()
        .ping()
        .await
        .map(Into::into)
}

// ─────────────────────────────────────────────────────────────────────────────
// 5b. list_environments (MCP-driven)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn list_environments(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Environment>, RanchErrorDto> {
    list_environments_inner(state.repository.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn list_environments_inner(
    connections: &dyn ConnectionRepository,
) -> Result<Vec<Environment>, RanchError> {
    let environments = connections.list_environments().await?;
    Ok(environments.into_iter().map(Into::into).collect())
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
    rename_environment_inner(
        &args.environment_id,
        &args.new_label,
        state.repository.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn rename_environment_inner(
    environment_id: &str,
    new_label: &str,
    connections: &dyn ConnectionRepository,
) -> Result<(), RanchError> {
    let trimmed = new_label.trim();
    if trimmed.is_empty() {
        return Err(RanchError::InvalidInput {
            reason: "Environment label cannot be empty".to_string(),
        });
    }
    connections
        .rename_environment(environment_id, trimmed)
        .await
}

// ─────────────────────────────────────────────────────────────────────────────
// 7. fetch_connection_by_id: shared helper
// ─────────────────────────────────────────────────────────────────────────────

/// Fetch a single connection by id, returning the same joined `Connection` shape
/// `list_connections_inner` produces. Returns `Ok(None)` if no row matches.
///
/// This helper exists because `update_connection` and `duplicate_connection`
/// both need to return the post-write view of the connection, including the
/// joined env/group fields.
pub async fn fetch_connection_by_id_inner(
    id: &str,
    connections: &dyn ConnectionRepository,
) -> Result<Option<Connection>, RanchError> {
    Ok(connections.get(id).await?.map(Into::into))
}

// ─────────────────────────────────────────────────────────────────────────────
// 8. update_connection
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateConnectionArgs {
    pub id: String,
    pub name: String,
    /// The kind of backend that serves it (`bullmq-redis`).
    pub backend: String,
    /// The whole endpoint: it replaces the saved one. Checked by the backend
    /// of its kind before anything is written.
    #[specta(type = JsonValue)]
    pub endpoint: serde_json::Value,
    /// `None` = leave the keychain entry untouched.
    /// `Some("")` = rejected as `InvalidInput` (the `Option<String>` is the
    /// "unchanged" sentinel, an empty string is presumed to be a UI bug).
    /// `Some(value)` = overwrite the keychain entry under the same connection id.
    pub password: Option<String>,
    /// SSH password (Option<String> with the same semantics as
    /// `password`):
    ///   - `None` → leave keychain entry untouched.
    ///   - `Some("")` → REJECT as `InvalidInput` (presumed UI bug).
    ///   - `Some(value)` → overwrite at `{id}::ssh`.
    ///
    /// A key-file tunnel, or an endpoint without the saved tunnel, deletes
    /// the `{id}::ssh` entry: this field is then ignored.
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
    update_connection_inner(
        args,
        state.repository.as_ref(),
        &state.keychain,
        state.backends.specs(),
        Some(state.connections.as_ref()),
    )
    .await
    .map_err(Into::into)
}

/// Save the edit, then drop the connection's actor: it resolved host,
/// credentials and lock state once, at spawn, so the next command
/// respawns it with what was just saved. Dropping also ends any session
/// unlock: the safe side.
pub async fn update_connection_inner(
    args: UpdateConnectionArgs,
    connections: &dyn ConnectionRepository,
    keychain: &Arc<dyn SecretStore>,
    specs: &BackendSpecs,
    registry: Option<&ConnectionRegistry>,
) -> Result<Connection, RanchError> {
    let id = args.id.clone();
    let updated = save_connection_update(args, connections, keychain, specs).await?;
    if let Some(registry) = registry {
        registry.shutdown(&id).await;
    }
    Ok(updated)
}

async fn save_connection_update(
    args: UpdateConnectionArgs,
    connections: &dyn ConnectionRepository,
    keychain: &Arc<dyn SecretStore>,
    specs: &BackendSpecs,
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
    reject_control_chars(&[
        ("The password", args.password.as_deref()),
        ("The SSH password", args.ssh_password.as_deref()),
    ])?;

    // The backend of the kind checks the endpoint before anything is written.
    let backend = BackendKind::new(args.backend.clone());
    let spec = specs.get(&backend)?;
    let endpoint = spec.validate(&Endpoint::new(args.endpoint.clone()))?;
    let ssh_tunnel = spec.tunnel(&endpoint);

    // "Leave blank to keep" keeps a password for the server it was saved
    // for, over the same route. Pointing the connection at another host,
    // turning TLS off, or going through another CA or SSH bastion with the
    // old password kept would hand it to them at the next connect; another
    // kind of backend is another server. A keychain that cannot say whether
    // a password is saved counts as one being there.
    let mut saved_had_tunnel = false;
    if let Some(saved) = connections.get(&args.id).await? {
        let same_server = saved.backend == backend && spec.same_server(&saved.endpoint, &endpoint);
        if args.password.is_none()
            && !same_server
            && !matches!(
                password_entry(&args.id).and_then(|entry| keychain.get(entry)),
                Ok(None)
            )
        {
            return Err(RanchError::InvalidInput {
                reason: "The server, or the route to it (CA, SSH bastion), changed: enter the Redis password again (the saved one is only sent the way it was saved for)".to_string(),
            });
        }
        let saved_tunnel = if saved.backend == backend {
            spec.tunnel(&saved.endpoint)
        } else {
            None
        };
        saved_had_tunnel = saved_tunnel.is_some();
        if args.ssh_password.is_none()
            && uses_ssh_password(ssh_tunnel.as_ref())
            && !same_ssh_endpoint(saved_tunnel.as_ref(), ssh_tunnel.as_ref())
            && !matches!(keychain.get(&ssh_keychain_name(&args.id)), Ok(None))
        {
            return Err(RanchError::InvalidInput {
                reason: "The SSH bastion changed: enter the SSH password again (the saved one is only sent to the bastion it was saved for)".to_string(),
            });
        }
    }

    // The secrets change first, outside any write transaction; a failed
    // edit puts them back as they were, as a failed add takes them back.
    let mut changes = SecretChanges::new(keychain.as_ref());
    let stored = update_secrets(&mut changes, &args, ssh_tunnel.as_ref(), saved_had_tunnel);
    if let Err(e) = stored {
        changes.undo();
        return Err(e);
    }

    let saved = connections
        .update(ConnectionEdit {
            id: args.id.clone(),
            name: args.name.clone(),
            backend,
            endpoint,
            group: args.group.clone().into(),
            environment: args.environment.clone().into(),
            locked: args.locked,
        })
        .await;
    match saved {
        Ok(connection) => Ok(connection.into()),
        Err(e) => {
            changes.undo();
            Err(e)
        }
    }
}

/// An edit's secrets: `None` leaves a password alone. The SSH password
/// follows the endpoint:
///   1. A password tunnel and `ssh_password.is_some()` → set the entry
///      (rotation / new password); `None` keeps it.
///   2. A key-file tunnel → delete the entry (orphan cleanup).
///   3. The saved tunnel removed → delete the entry.
fn update_secrets(
    changes: &mut SecretChanges<'_>,
    args: &UpdateConnectionArgs,
    ssh_tunnel: Option<&ssh_config::SshTunnelConfig>,
    saved_had_tunnel: bool,
) -> Result<(), RanchError> {
    if let Some(pw) = &args.password {
        changes.replace(password_entry(&args.id)?.to_string(), pw)?;
    }
    let ssh_kc = ssh_keychain_name(&args.id);
    if uses_ssh_password(ssh_tunnel) {
        if let Some(pw) = &args.ssh_password {
            changes.replace(ssh_kc, pw)?;
        }
    } else if ssh_tunnel.is_some() || saved_had_tunnel {
        match changes.delete(ssh_kc) {
            // Warn but proceed (matches remove_connection's pattern).
            Err(e @ RanchError::KeychainUnavailable { .. }) => {
                tracing::warn!(error = %e, connection_id = %args.id, "keychain unavailable while clearing SSH password; proceeding");
            }
            other => other?,
        }
    }
    Ok(())
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
    duplicate_connection_inner(&args.id, state.repository.as_ref(), &state.keychain)
        .await
        .map_err(Into::into)
}

pub async fn duplicate_connection_inner(
    source_id: &str,
    connections: &dyn ConnectionRepository,
    keychain: &Arc<dyn SecretStore>,
) -> Result<Connection, RanchError> {
    let new_id = uuid::Uuid::now_v7().to_string();
    // Before the copy's transaction, as for an add: the credential, the SSH
    // password and the host-key pins go to new keychain entries, each
    // skipped when the source had none, and are taken back if the copy
    // cannot be saved.
    let mut changes = SecretChanges::new(keychain.as_ref());
    if let Err(e) = copy_secrets(&mut changes, source_id, &new_id) {
        changes.undo();
        return Err(e);
    }
    match connections.duplicate(source_id, &new_id).await {
        Ok(connection) => Ok(connection.into()),
        Err(e) => {
            changes.undo();
            Err(e)
        }
    }
}

/// A connection's keychain entries, under a copy's id.
fn copy_secrets(
    changes: &mut SecretChanges<'_>,
    source_id: &str,
    new_id: &str,
) -> Result<(), RanchError> {
    let entries = [
        (
            password_entry(source_id)?.to_string(),
            password_entry(new_id)?.to_string(),
        ),
        (ssh_keychain_name(source_id), ssh_keychain_name(new_id)),
        (
            ssh_hostkey_keychain_name(source_id),
            ssh_hostkey_keychain_name(new_id),
        ),
        (
            ssh_jump_hostkey_keychain_name(source_id),
            ssh_jump_hostkey_keychain_name(new_id),
        ),
    ];
    for (from, to) in entries {
        if let Some(secret) = changes.keychain.get(&from)? {
            changes.create(to, &secret)?;
        }
    }
    Ok(())
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
        state.repository.as_ref(),
        &state.keychain,
        Some(state.connections.as_ref()),
    )
    .await
    .map_err(Into::into)
}

pub async fn remove_connection_inner(
    id: &str,
    connections: &dyn ConnectionRepository,
    keychain: &Arc<dyn SecretStore>,
    registry: Option<&ConnectionRegistry>,
) -> Result<(), RanchError> {
    if connections.get(id).await?.is_none() {
        return Err(RanchError::ConnectionNotFound { id: id.to_string() });
    }
    let password = password_entry(id)?;

    // Delete keychain entries FIRST. Rationale: leaking a stale credential in
    // the keychain is preferable to leaving a row pointing at a missing one
    // (which would surface as KeychainUnavailable on every read).
    //
    // Clean up FOUR entries:
    //   1. Redis password at `{id}`.
    //   2. SSH password at `{id}::ssh`.
    //   3. SSH hostkey pin at `{id}::ssh::hostkey`.
    //   4. The jump host's pin.
    // Each delete is independent: if one fails with KeychainUnavailable we log
    // + continue (matches the `remove_connection` pattern).
    let ssh_kc = ssh_keychain_name(id);
    let ssh_hostkey_kc = ssh_hostkey_keychain_name(id);
    let jump_hostkey_kc = ssh_jump_hostkey_keychain_name(id);
    for entry_name in [
        password,
        ssh_kc.as_str(),
        ssh_hostkey_kc.as_str(),
        jump_hostkey_kc.as_str(),
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

    connections.delete(id).await?;

    // Shut down the live actor (if any) AFTER the SQLite row
    // is gone. Order matters: if the DELETE failed we want the actor to
    // stay alive (caller may retry). Once the row is deleted, the actor
    // is orphaned: drop it cleanly. Tests pass `None` to skip this.
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
    use crate::adapters::bullmq_redis::endpoint::BULLMQ_REDIS;

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
        // `%40` = `@`. Managed Redis passwords often contain @.
        let r = parse_redis_url_inner("redis://default:p%40ss@h:6379".into())
            .await
            .unwrap();
        assert_eq!(r.password.as_deref(), Some("p@ss"));
    }

    /// A Redis endpoint as the form sends it.
    fn redis(host: &str, port: u16) -> BullmqRedisEndpoint {
        BullmqRedisEndpoint {
            host: host.into(),
            port,
            db: 0,
            username: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
        }
    }

    fn test_args(endpoint: &BullmqRedisEndpoint) -> TestConnectionArgs {
        TestConnectionArgs {
            backend: BULLMQ_REDIS.into(),
            endpoint: endpoint.json(),
            password: None,
            ssh_password: None,
            saved_connection_id: None,
        }
    }

    /// A saved connection with this endpoint, as the repository lists it.
    fn saved(id: &str, endpoint: &BullmqRedisEndpoint) -> hierarchy::Connection {
        hierarchy::Connection {
            id: id.into(),
            environment_id: "e".into(),
            group_id: "g".into(),
            name: "saved".into(),
            backend: BackendKind::new(BULLMQ_REDIS),
            endpoint: Endpoint::new(endpoint.json()),
            env_color: "env-local".into(),
            env_name: "Local".into(),
            env_label: "Local".into(),
            group_name: "Lumen Studios".into(),
            created_at: "t0".into(),
            mcp_readonly: None,
            sidebar_collapsed: false,
        }
    }

    #[tokio::test]
    async fn test_connection_returns_cannot_reach_for_dead_port() {
        // 127.0.0.1:1 is reserved/unused: connection refused fast.
        // (An injected keychain: the test runs under both feature sets.)
        let r = test_connection_inner_with_keychain(
            test_args(&redis("127.0.0.1", 1)),
            &crate::adapters::bullmq_redis::specs(),
            Arc::new(crate::adapters::secrets::InMemoryKeychain::new()),
        )
        .await
        .unwrap();
        assert!(!r.success);
        assert!(r.error.is_some());
        // Either CannotReach or Timeout depending on platform; both are acceptable
        // signals that the user can't reach the host.
        match r.error.unwrap() {
            TestConnectionError::CannotReach { .. } | TestConnectionError::Timeout => {}
            other => panic!("expected CannotReach or Timeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_connection_checks_the_endpoint_and_its_kind() {
        let keychain: Arc<dyn SecretStore> =
            Arc::new(crate::adapters::secrets::InMemoryKeychain::new());
        let other_kind = TestConnectionArgs {
            backend: "memory".into(),
            ..test_args(&redis("127.0.0.1", 1))
        };
        let bad_ca = TestConnectionArgs {
            endpoint: BullmqRedisEndpoint {
                tls: true,
                tls_custom_ca: Some("not a pem".into()),
                ..redis("127.0.0.1", 1)
            }
            .json(),
            ..test_args(&redis("127.0.0.1", 1))
        };
        for args in [other_kind, bad_ca] {
            let err = test_connection_inner_with_keychain(
                args,
                &crate::adapters::bullmq_redis::specs(),
                keychain.clone(),
            )
            .await
            .unwrap_err();
            assert!(matches!(err, RanchError::InvalidInput { .. }), "{err:?}");
        }
    }

    #[test]
    fn a_blank_password_on_edit_comes_from_the_keychain() {
        let keychain = crate::adapters::secrets::InMemoryKeychain::new();
        keychain.set("c1", "saved").unwrap();
        keychain.set(&ssh_keychain_name("c1"), "ssh-saved").unwrap();
        let specs = crate::adapters::bullmq_redis::specs();
        let endpoint = BullmqRedisEndpoint {
            ssh_tunnel: Some(SshTunnelConfig {
                host: "b".into(),
                port: 22,
                user: "u".into(),
                auth: SshAuthMethod::Password,
                jump_host: None,
            }),
            ..redis("h", 6379)
        };
        let base = TestConnectionArgs {
            saved_connection_id: Some("c1".into()),
            ..test_args(&endpoint)
        };
        let saved = saved("c1", &endpoint);
        let filled = with_saved_secrets(clone_args(&base), Some(&saved), &specs, &keychain);
        assert_eq!(filled.password.as_deref(), Some("saved"));
        assert_eq!(filled.ssh_password.as_deref(), Some("ssh-saved"));

        // A typed password wins; a new connection has nothing to fall back on.
        let typed = TestConnectionArgs {
            password: Some("typed".into()),
            ..clone_args(&base)
        };
        assert_eq!(
            with_saved_secrets(typed, Some(&saved), &specs, &keychain)
                .password
                .as_deref(),
            Some("typed")
        );
        let fresh = TestConnectionArgs {
            saved_connection_id: None,
            ..clone_args(&base)
        };
        assert_eq!(
            with_saved_secrets(fresh, Some(&saved), &specs, &keychain).password,
            None
        );
        // An id that matches no saved connection: nothing to fall back on.
        assert_eq!(
            with_saved_secrets(clone_args(&base), None, &specs, &keychain).password,
            None
        );
    }

    #[test]
    fn a_saved_secret_never_goes_to_another_endpoint() {
        let keychain = crate::adapters::secrets::InMemoryKeychain::new();
        keychain.set("c1", "saved").unwrap();
        keychain.set(&ssh_keychain_name("c1"), "ssh-saved").unwrap();
        let specs = crate::adapters::bullmq_redis::specs();
        let bastion = SshTunnelConfig {
            host: "bastion.internal".into(),
            port: 22,
            user: "u".into(),
            auth: SshAuthMethod::Password,
            jump_host: None,
        };
        let endpoint = BullmqRedisEndpoint {
            tls: true,
            ssh_tunnel: Some(bastion.clone()),
            ..redis("redis.prod.internal", 6380)
        };
        let saved = saved("c1", &endpoint);
        let with = |ep: BullmqRedisEndpoint| TestConnectionArgs {
            saved_connection_id: Some("c1".into()),
            ..test_args(&ep)
        };
        // Another host, another port, TLS turned off, another kind of
        // backend: no saved password.
        for args in [
            with(BullmqRedisEndpoint {
                host: "attacker.example".into(),
                ..endpoint.clone()
            }),
            with(BullmqRedisEndpoint {
                port: 6379,
                ..endpoint.clone()
            }),
            with(BullmqRedisEndpoint {
                tls: false,
                ..endpoint.clone()
            }),
            TestConnectionArgs {
                backend: "memory".into(),
                ..with(endpoint.clone())
            },
        ] {
            assert_eq!(
                with_saved_secrets(args, Some(&saved), &specs, &keychain).password,
                None
            );
        }
        // Host names compare without case.
        let upper = with(BullmqRedisEndpoint {
            host: "REDIS.prod.internal".into(),
            ..endpoint.clone()
        });
        assert_eq!(
            with_saved_secrets(upper, Some(&saved), &specs, &keychain)
                .password
                .as_deref(),
            Some("saved")
        );
        // Another bastion or user: no saved SSH password.
        for ssh in [
            SshTunnelConfig {
                host: "evil.example".into(),
                ..bastion.clone()
            },
            SshTunnelConfig {
                user: "root".into(),
                ..bastion.clone()
            },
            SshTunnelConfig {
                port: 2222,
                ..bastion.clone()
            },
        ] {
            let args = with(BullmqRedisEndpoint {
                ssh_tunnel: Some(ssh),
                ..endpoint.clone()
            });
            assert_eq!(
                with_saved_secrets(args, Some(&saved), &specs, &keychain).ssh_password,
                None
            );
        }
    }

    #[test]
    fn a_backend_lists_its_form() {
        let info = list_backends_inner(&crate::adapters::bullmq_redis::specs()).remove(0);
        assert_eq!(
            (info.kind.as_str(), info.label.as_str()),
            ("bullmq-redis", "BullMQ on Redis")
        );
        let secrets: Vec<_> = info
            .endpoint_schema
            .fields
            .iter()
            .filter(|f| f.kind == FieldKind::Secret)
            .map(|f| f.key.as_str())
            .collect();
        assert_eq!(secrets, ["password"]);
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["endpointSchema"]["fields"][1]["kind"], "number");
        assert_eq!(json["endpointSchema"]["fields"][1]["default"], 6379);
        // What it can do is its spec's, the BullMQ states described.
        assert_eq!(json["capabilities"]["schedules"]["pause"], true);
        let states: Vec<_> = info
            .capabilities
            .job_states
            .iter()
            .map(|s| (s.code.as_str(), s.common))
            .collect();
        assert_eq!(
            states,
            [
                ("waiting", true),
                ("active", true),
                ("completed", true),
                ("failed", true),
                ("delayed", true),
                ("prioritized", false),
                ("waitingChildren", false),
                ("stalled", false),
                ("paused", true),
            ]
        );
        assert_eq!(
            json["capabilities"]["jobStates"][6]["label"],
            "Waiting children"
        );
    }

    fn clone_args(a: &TestConnectionArgs) -> TestConnectionArgs {
        TestConnectionArgs {
            backend: a.backend.clone(),
            endpoint: a.endpoint.clone(),
            password: a.password.clone(),
            ssh_password: a.ssh_password.clone(),
            saved_connection_id: a.saved_connection_id.clone(),
        }
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
    set_connection_collapsed_inner(
        &args.connection_id,
        args.collapsed,
        state.repository.as_ref(),
    )
    .await
    .map_err(Into::into)
}

pub async fn set_connection_collapsed_inner(
    connection_id: &str,
    collapsed: bool,
    connections: &dyn ConnectionRepository,
) -> Result<(), RanchError> {
    connections.set_collapsed(connection_id, collapsed).await
}
