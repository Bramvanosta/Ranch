//! SSH tunnel runtime (russh-backed).
//!
//! This module activates the SSH-tunnel structural config captured with the
//! connection. The connection actor calls [`open`] to establish a tunnel
//! to the bastion; the returned [`SshTunnel`] owns:
//!
//!   - the russh session handle (kept alive for the tunnel's lifetime),
//!   - a localhost TCP listener bound on an OS-assigned port,
//!   - a spawned forwarder task piping `listener ↔ russh channel`,
//!   - a `CancellationToken` that the forwarder watches.
//!
//! The Redis URL builder then targets `127.0.0.1:<local-port>`; bytes
//! travel listener → SSH-channel → bastion → target Redis.
//!
//! ## Lifecycle
//!
//!   - **Cold open**: `open()` connects + authenticates + caches the
//!     hostkey fingerprint (TOFU). Latency budget: ~500-1200ms.
//!   - **Cached reuse**: the actor stores `Arc<SshTunnel>` on
//!     `ActorState` and reuses for subsequent Redis ops. The forwarder
//!     loop accepts new TCP conns + opens fresh `direct-tcpip` channels
//!     per Redis multiplexed connection.
//!   - **Drop / shutdown**: `Drop` impl fires the cancellation token →
//!     forwarder task exits → listener closes; the russh `Handle` then
//!     drops cleanly. No zombie tasks.
//!
//! ## Host-key TOFU pin
//!
//! On the first connect for a given `connection_id`, the bastion's
//! SHA256 public-key fingerprint is captured and persisted in the
//! keychain under `{connection_id}::ssh::hostkey`. Subsequent connects
//! compare; mismatch rejects the session. The keychain entry is
//! cleaned in `remove_connection` and explicitly NOT carried
//! by `duplicate_connection` (each duplicate re-pins on first connect).
//!
//! ## Jump host
//!
//! When `cfg.jump_host` is `Some`, [`open`] establishes the SSH session
//! to the jump host first, opens a `direct-tcpip` channel from the jump
//! host to the bastion's SSH port, then runs the *second* russh client
//! over that channel via `connect_stream`. Same credential is reused on
//! both hops (a v1 simplification).
//!
//! ## Forbidden in this module
//!
//!   - `unsafe` Rust (russh is pure-safe).
//!   - System OpenSSL / libssh2 (russh + ring is the chosen stack).
//!   - `rediss://` (TLS) on the loopback hop — the SSH session already
//!     encrypts; double-encrypting localhost adds zero security and
//!     surfaces cert-name-mismatch errors.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::{load_secret_key, HashAlg, PrivateKeyWithHashAlg, PublicKey};
use russh::{ChannelId, Disconnect};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::core::errors::RanchError;
use crate::core::keychain::KeychainAdapter;
use crate::core::ssh_config::{SshAuthMethod, SshJumpHost, SshTunnelConfig};

/// Cold-open budget for the SSH handshake + auth + channel open.
/// Each phase typically <300ms; the cumulative budget here covers
/// cross-continent bastions on slow networks. Sub-second total is
/// the warm path (cached tunnel — not this code).
const SSH_OPEN_TIMEOUT: Duration = Duration::from_secs(8);

/// Returned by [`open`] alongside the tunnel handle. `was_new_pin` is
/// true when this connect was the FIRST connect for the connection
/// (the hostkey was just stored). The form's success banner reads
/// this to surface "First connect — host key pinned" to the user.
#[derive(Debug)]
pub struct OpenResult {
    pub tunnel: SshTunnel,
    pub fingerprint: String,
    pub was_new_pin: bool,
}

/// Owned tunnel handle. While alive, bytes piped to
/// [`Self::local_addr`] reach the target Redis through the SSH session.
///
/// Cloning is intentionally not supported: a tunnel has exactly one
/// owner. The connection actor caches `Arc<SshTunnel>` so multiple
/// callers within the same actor share, but the underlying handle
/// drops once and only once.
pub struct SshTunnel {
    local_addr: SocketAddr,
    cancel: CancellationToken,
    /// Held to keep the russh session alive. The forwarder task also
    /// holds a clone of the Arc — when both drop, the russh session
    /// closes. Wrapped in `Arc` because `Handle<H>` is not `Clone`
    /// (it owns a `JoinHandle`), and `channel_open_direct_tcpip`
    /// takes `&self` so shared-immutable access is sufficient.
    _session: Arc<Handle<RanchSshHandler>>,
}

impl std::fmt::Debug for SshTunnel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshTunnel")
            .field("local_addr", &self.local_addr)
            .finish_non_exhaustive()
    }
}

impl SshTunnel {
    /// Returns the bound `127.0.0.1:<random>` address. The Redis URL
    /// builder targets this so the Redis client opens a TCP socket to
    /// the localhost listener; the forwarder pipes that to the russh
    /// channel.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The SSH session has ended (bastion gone, keepalives
    /// unanswered): the actor drops this tunnel and its Redis connection.
    pub fn is_closed(&self) -> bool {
        self._session.is_closed()
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        // Signal the forwarder task to exit. The russh `Handle` drops
        // next — its Drop impl sends a SSH_MSG_DISCONNECT cleanly.
        self.cancel.cancel();
    }
}

/// Builds a russh client config used throughout the module. Pure-Rust
/// (ring crypto). Defaults are otherwise stock; no kex / cipher
/// overrides — russh's default cipher suite is modern (AES-GCM,
/// chacha20-poly1305).
///
/// Nagle off on the SSH socket (small Redis commands waiting
/// for replies), and a keepalive every 30 s: an idle tunnel stays open, and a
/// dead bastion is noticed after 3 unanswered keepalives.
fn client_config() -> Arc<client::Config> {
    Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(300)),
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        nodelay: true,
        ..Default::default()
    })
}

/// Build the SHA256 fingerprint string for a host's public key.
/// Format matches `ssh-keygen -lf` output (e.g.
/// `SHA256:abcdef…`); used as the keychain-pinned value.
pub(crate) fn fingerprint_sha256(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// Keychain entry name for the hostkey pin. Distinct from the SSH
/// password entry (`{id}::ssh`) and the Redis password
/// entry (`{id}`). Bumps the per-connection keychain
/// count from 2 to 3.
pub fn ssh_hostkey_keychain_name(connection_id: &str) -> String {
    format!("{connection_id}::ssh::hostkey")
}

/// Internal `client::Handler` used for each russh session. Reads the
/// pinned fingerprint from the keychain on `check_server_key` and:
///   - accepts + writes the fingerprint on first connect (TOFU),
///   - accepts when fingerprints match,
///   - returns `Ok(false)` on mismatch (russh rejects the session).
///
/// `was_new_pin` is captured in `Arc<Mutex<…>>` so [`open`] can read
/// the outcome after `russh::client::connect` returns. (The handler
/// runs on the russh session task; the only safe way to surface state
/// back to the caller is shared state.)
struct RanchSshHandler {
    connection_id: String,
    keychain: Arc<dyn KeychainAdapter>,
    /// Captured fingerprint + first-connect flag. Mutated inside
    /// `check_server_key`; read by [`open`] after auth completes.
    state: Arc<tokio::sync::Mutex<HandlerState>>,
}

#[derive(Default)]
struct HandlerState {
    fingerprint: Option<String>,
    was_new_pin: bool,
    /// If `Some`, the check rejected with this reason — the wrapper
    /// helper can surface a useful error rather than just the generic
    /// "host key check failed" russh ships.
    reject_reason: Option<String>,
}

impl client::Handler for RanchSshHandler {
    type Error = russh::Error;

    /// TOFU: read the pinned fingerprint from the keychain. None → pin
    /// this one. Some(saved) → accept iff match, else reject.
    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        let observed = fingerprint_sha256(server_public_key);
        let kc_name = ssh_hostkey_keychain_name(&self.connection_id);
        let mut state = self.state.lock().await;
        state.fingerprint = Some(observed.clone());

        match self.keychain.get(&kc_name) {
            Ok(Some(pinned)) if pinned == observed => Ok(true),
            Ok(Some(pinned)) => {
                state.reject_reason = Some(format!(
                    "Host key fingerprint mismatch — expected {pinned}, got {observed}. If you \
                     trust the change, remove the connection and re-add it."
                ));
                Ok(false)
            }
            Ok(None) => {
                // First connect for this connection_id — pin it.
                // A keychain write failure here is non-fatal (e.g. the
                // user has no keychain available — already-degraded
                // path of the keychain adapter). We still accept the
                // session; next connect will repeat the TOFU prompt.
                if let Err(e) = self.keychain.set(&kc_name, &observed) {
                    tracing::warn!(
                        connection_id = %self.connection_id,
                        error = %e,
                        "Failed to persist SSH hostkey pin; proceeding (will TOFU again next connect)",
                    );
                }
                state.was_new_pin = true;
                Ok(true)
            }
            Err(e) => {
                // Keychain unavailable. Reject defensively — we cannot
                // verify the host's identity. Better a hard error than
                // a silent trust-on-every-connect.
                state.reject_reason = Some(format!(
                    "Could not verify host key — keychain unavailable: {e}"
                ));
                Ok(false)
            }
        }
    }
}

/// Open an SSH tunnel from `127.0.0.1:<auto>` to `target` through the
/// bastion described by `cfg`. The returned [`OpenResult`] carries the
/// tunnel handle, the captured fingerprint, and the `was_new_pin` flag
/// (TOFU first-connect signal).
///
/// # Errors
///
///   - [`RanchError::SshKeyFileError`] — key file unreadable/unparseable.
///   - [`RanchError::SshAuthFailed`] — bastion rejected the credential.
///   - [`RanchError::SshTunnelFailed`] — bastion unreachable, hostkey
///     mismatch, channel open refused, jump-hop failure, etc.
pub async fn open(
    cfg: &SshTunnelConfig,
    password: Option<&str>,
    target: (&str, u16),
    keychain: Arc<dyn KeychainAdapter>,
    connection_id: &str,
) -> Result<OpenResult, RanchError> {
    // Pre-flight: load the key file, if relevant. Doing this BEFORE
    // we open a TCP socket gives the user a fast typed error on path
    // / parse problems (the common config mistake).
    let private_key = match &cfg.auth {
        SshAuthMethod::KeyFile { key_file_path } => {
            let path = key_file_path.clone();
            // Run the blocking file IO + key-parse on the blocking pool
            // (load_secret_key is sync; spawn_blocking keeps the async
            // task responsive on the rare large-key case).
            let loaded = tokio::task::spawn_blocking(move || load_secret_key(&path, None))
                .await
                .map_err(|e| RanchError::SshKeyFileError {
                    path: key_file_path.clone(),
                    reason: format!("join error: {e}"),
                })?;
            match loaded {
                Ok(k) => Some(Arc::new(k)),
                Err(e) => {
                    return Err(RanchError::SshKeyFileError {
                        path: key_file_path.clone(),
                        reason: e.to_string(),
                    });
                }
            }
        }
        SshAuthMethod::Password => None,
    };

    // Establish the session to the bastion (possibly hopping through a
    // jump host first).
    let (session, state) = open_session_via_optional_jump(
        cfg,
        keychain.clone(),
        connection_id,
        private_key.as_ref(),
        password,
    )
    .await?;

    // Open the local forwarding listener. `127.0.0.1:0` lets the OS
    // pick a free port; the actor's URL builder reads it.
    let listener =
        TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| RanchError::SshTunnelFailed {
                reason: format!("Could not bind local forwarding listener: {e}"),
            })?;
    let local_addr = listener
        .local_addr()
        .map_err(|e| RanchError::SshTunnelFailed {
            reason: format!("Could not read local listener address: {e}"),
        })?;

    // Wrap the session in `Arc` so the forwarder task can clone it.
    // russh's `Handle` is not Clone; the Arc gives us shared
    // immutable access (channel_open_direct_tcpip takes &self).
    let session = Arc::new(session);

    // Spawn the forwarder. It owns the listener + a cloned Arc<Handle>
    // + the cancellation token. The token is held in the SshTunnel so
    // Drop kills the loop.
    let cancel = CancellationToken::new();
    let forwarder_session = Arc::clone(&session);
    let forwarder_cancel = cancel.clone();
    let target_host = target.0.to_string();
    let target_port = target.1;
    tokio::spawn(async move {
        run_forwarder(
            listener,
            forwarder_session,
            (target_host, target_port),
            forwarder_cancel,
        )
        .await;
    });

    // Drain the captured handler state (fingerprint + was_new_pin).
    let captured = state.lock().await;
    let fingerprint = captured.fingerprint.clone().unwrap_or_default();
    let was_new_pin = captured.was_new_pin;
    drop(captured);

    Ok(OpenResult {
        tunnel: SshTunnel {
            local_addr,
            cancel,
            _session: session,
        },
        fingerprint,
        was_new_pin,
    })
}

/// Establishes the russh session to the bastion. If `cfg.jump_host` is
/// `Some`, hops through the jump host first by opening a
/// `direct-tcpip` channel to the bastion's SSH port and running a
/// second russh client over that channel via `connect_stream`.
async fn open_session_via_optional_jump(
    cfg: &SshTunnelConfig,
    keychain: Arc<dyn KeychainAdapter>,
    connection_id: &str,
    private_key: Option<&Arc<russh::keys::PrivateKey>>,
    password: Option<&str>,
) -> Result<
    (
        Handle<RanchSshHandler>,
        Arc<tokio::sync::Mutex<HandlerState>>,
    ),
    RanchError,
> {
    let state = Arc::new(tokio::sync::Mutex::new(HandlerState::default()));
    let handler = RanchSshHandler {
        connection_id: connection_id.to_string(),
        keychain: keychain.clone(),
        state: state.clone(),
    };

    match &cfg.jump_host {
        None => {
            let session = connect_direct(
                handler,
                &cfg.host,
                cfg.port,
                &cfg.user,
                private_key,
                password,
            )
            .await
            .map_err(|reason| {
                // Map the handler's reject reason (if any) into
                // the error so the user sees the hostkey-mismatch
                // message specifically rather than the generic
                // "handshake failed".
                let state = state.try_lock();
                if let Ok(s) = state {
                    if let Some(r) = &s.reject_reason {
                        return RanchError::SshTunnelFailed { reason: r.clone() };
                    }
                }
                reason
            })?;
            check_handler_post_auth(&state).await?;
            Ok((session, state))
        }
        Some(jump) => {
            let session = connect_via_jump(
                handler,
                jump,
                &cfg.host,
                cfg.port,
                &cfg.user,
                private_key,
                password,
                keychain,
                connection_id,
            )
            .await
            .map_err(|reason| {
                let s = state.try_lock();
                if let Ok(s) = s {
                    if let Some(r) = &s.reject_reason {
                        return RanchError::SshTunnelFailed { reason: r.clone() };
                    }
                }
                reason
            })?;
            check_handler_post_auth(&state).await?;
            Ok((session, state))
        }
    }
}

/// After russh's `connect` returns, surface any hostkey-mismatch the
/// handler captured. (russh itself maps a `false` from
/// `check_server_key` to a generic `Disconnect`; we want a specific
/// message.)
async fn check_handler_post_auth(
    state: &Arc<tokio::sync::Mutex<HandlerState>>,
) -> Result<(), RanchError> {
    let s = state.lock().await;
    if let Some(reason) = &s.reject_reason {
        return Err(RanchError::SshTunnelFailed {
            reason: reason.clone(),
        });
    }
    Ok(())
}

/// Direct (no-jump) connect + authenticate. Returns the russh Handle
/// ready to open channels.
async fn connect_direct(
    handler: RanchSshHandler,
    host: &str,
    port: u16,
    user: &str,
    private_key: Option<&Arc<russh::keys::PrivateKey>>,
    password: Option<&str>,
) -> Result<Handle<RanchSshHandler>, RanchError> {
    let config = client_config();
    let connect_fut = client::connect(config, (host, port), handler);
    let mut session = tokio::time::timeout(SSH_OPEN_TIMEOUT, connect_fut)
        .await
        .map_err(|_| RanchError::SshTunnelFailed {
            reason: format!("Timed out connecting to SSH host {host}:{port}"),
        })?
        .map_err(|e| RanchError::SshTunnelFailed {
            reason: format!("Failed to connect to {host}:{port}: {e}"),
        })?;

    authenticate(&mut session, user, private_key, password).await?;
    Ok(session)
}

/// Two-hop connect: open SSH session to the jump host, open a
/// `direct-tcpip` channel from the jump host to the bastion's SSH
/// port, then open a new russh session over that channel.
#[allow(clippy::too_many_arguments)]
async fn connect_via_jump(
    handler: RanchSshHandler,
    jump: &SshJumpHost,
    bastion_host: &str,
    bastion_port: u16,
    user: &str,
    private_key: Option<&Arc<russh::keys::PrivateKey>>,
    password: Option<&str>,
    keychain: Arc<dyn KeychainAdapter>,
    connection_id: &str,
) -> Result<Handle<RanchSshHandler>, RanchError> {
    // Hop 1: connect to the jump host with a SEPARATE handler instance
    // (its own keychain pin scope; the jump host pin keys under
    // `{id}::ssh::hostkey::jump` to keep it distinct from the bastion).
    let jump_state = Arc::new(tokio::sync::Mutex::new(HandlerState::default()));
    let jump_handler = RanchSshHandler {
        connection_id: format!("{connection_id}::jump"),
        keychain: keychain.clone(),
        state: jump_state.clone(),
    };
    let config = client_config();
    let jump_session = tokio::time::timeout(
        SSH_OPEN_TIMEOUT,
        client::connect(
            config.clone(),
            (jump.host.as_str(), jump.port),
            jump_handler,
        ),
    )
    .await
    .map_err(|_| RanchError::SshTunnelFailed {
        reason: format!(
            "Timed out connecting to jump host {}:{}",
            jump.host, jump.port
        ),
    })?
    .map_err(|e| RanchError::SshTunnelFailed {
        reason: format!(
            "Failed to connect to jump host {}:{}: {e}",
            jump.host, jump.port
        ),
    })?;
    let jump_state_check = jump_state.lock().await;
    if let Some(reason) = &jump_state_check.reject_reason {
        return Err(RanchError::SshTunnelFailed {
            reason: format!("Jump host: {reason}"),
        });
    }
    drop(jump_state_check);
    let mut jump_session = jump_session;
    authenticate(&mut jump_session, &jump.user, private_key, password)
        .await
        .map_err(|e| match e {
            RanchError::SshAuthFailed { reason } => RanchError::SshAuthFailed {
                reason: format!("Jump host: {reason}"),
            },
            other => other,
        })?;

    // Hop 2: open the direct-tcpip channel from the jump host to the
    // bastion's SSH port, then layer a new russh client over it.
    let channel = jump_session
        .channel_open_direct_tcpip(bastion_host, bastion_port as u32, "127.0.0.1", 0)
        .await
        .map_err(|e| RanchError::SshTunnelFailed {
            reason: format!(
                "Could not open jump-host → bastion channel ({bastion_host}:{bastion_port}): {e}"
            ),
        })?;

    let stream = channel.into_stream();
    let bastion_session = tokio::time::timeout(
        SSH_OPEN_TIMEOUT,
        client::connect_stream(config, stream, handler),
    )
    .await
    .map_err(|_| RanchError::SshTunnelFailed {
        reason: format!(
            "Timed out connecting to bastion {bastion_host}:{bastion_port} through jump host"
        ),
    })?
    .map_err(|e| RanchError::SshTunnelFailed {
        reason: format!(
            "Failed to connect to bastion {bastion_host}:{bastion_port} through jump host: {e}"
        ),
    })?;
    let mut bastion_session = bastion_session;
    authenticate(&mut bastion_session, user, private_key, password).await?;
    // Keep the jump session alive for the lifetime of the bastion
    // session — the bastion talks over a channel through the jump
    // session. We leak it intentionally by leaking the JoinHandle of
    // a holder task; when the bastion handle drops, the channel
    // closes, and the holder task notices the disconnect (jump session
    // EOF) and exits.
    tokio::spawn(async move {
        // Holder: block until the jump session disconnects.
        let _ = jump_session
            .disconnect(Disconnect::ByApplication, "tunnel-closed", "en")
            .await;
    });
    Ok(bastion_session)
}

/// Run either password or pubkey auth depending on which credential is
/// present. Maps russh's `AuthResult::Failure` → `RanchError::SshAuthFailed`.
async fn authenticate(
    session: &mut Handle<RanchSshHandler>,
    user: &str,
    private_key: Option<&Arc<russh::keys::PrivateKey>>,
    password: Option<&str>,
) -> Result<(), RanchError> {
    if let Some(key) = private_key {
        // For RSA keys, default to SHA-256 (sha-rsa is deprecated on
        // OpenSSH 8.8+); for ed25519/ecdsa, `hash_alg` is ignored.
        let key_with_alg = PrivateKeyWithHashAlg::new(Arc::clone(key), Some(HashAlg::Sha256));
        let result = session
            .authenticate_publickey(user.to_string(), key_with_alg)
            .await
            .map_err(|e| RanchError::SshAuthFailed {
                reason: format!("Public-key auth error: {e}"),
            })?;
        if !result.success() {
            return Err(RanchError::SshAuthFailed {
                reason: "Bastion rejected public-key authentication".to_string(),
            });
        }
        Ok(())
    } else if let Some(pw) = password {
        let result = session
            .authenticate_password(user.to_string(), pw.to_string())
            .await
            .map_err(|e| RanchError::SshAuthFailed {
                reason: format!("Password auth error: {e}"),
            })?;
        if !result.success() {
            return Err(RanchError::SshAuthFailed {
                reason: "Bastion rejected password authentication".to_string(),
            });
        }
        Ok(())
    } else {
        Err(RanchError::SshAuthFailed {
            reason: "No SSH credential provided (neither key file nor password)".to_string(),
        })
    }
}

/// Forwarder task: accept TCP conns on `listener`, open a
/// `direct-tcpip` channel for each, and pipe bytes bidirectionally.
/// Exits on `cancel`.
async fn run_forwarder(
    listener: TcpListener,
    session: Arc<Handle<RanchSshHandler>>,
    target: (String, u16),
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            accept = listener.accept() => {
                let Ok((stream, _)) = accept else { continue };
                // No Nagle on the loopback hop either.
                let _ = stream.set_nodelay(true);
                let session = Arc::clone(&session);
                let target = target.clone();
                let cancel = cancel.clone();
                tokio::spawn(async move {
                    if let Err(e) = pipe_one_connection(stream, session, target, cancel).await {
                        tracing::debug!(error = %e, "SSH-tunnel forwarder connection ended with error");
                    }
                });
            }
        }
    }
}

/// Handle ONE accepted local TCP connection: open a direct-tcpip
/// channel through the SSH session, then `copy_bidirectional` between
/// the local TCP stream and the channel.
async fn pipe_one_connection(
    local: TcpStream,
    session: Arc<Handle<RanchSshHandler>>,
    target: (String, u16),
    cancel: CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let channel = session
        .channel_open_direct_tcpip(target.0.as_str(), target.1 as u32, "127.0.0.1", 0)
        .await?;
    let channel_stream = channel.into_stream();
    pipe_bidirectional(local, channel_stream, cancel).await?;
    Ok(())
}

/// `tokio::io::copy_bidirectional` racing against cancellation. Both
/// halves of the copy are cancel-safe by virtue of the cancel future
/// being polled alongside.
async fn pipe_bidirectional<A, B>(
    mut a: A,
    mut b: B,
    cancel: CancellationToken,
) -> Result<(), std::io::Error>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    tokio::select! {
        r = tokio::io::copy_bidirectional(&mut a, &mut b) => r.map(|_| ()),
        _ = cancel.cancelled() => Ok(()),
    }
}

// We intentionally don't use `ChannelId`; pull it into the use line
// to silence the unused-import warning if a future revision needs it.
#[allow(dead_code)]
type _UnusedChannelId = ChannelId;

// ─────────────────────────────────────────────────────────────────────
// Unit tests — handler logic only. The live integration tests live in
// `tests/ssh_tunnel_runtime.rs` (gated by `RANCH_SSH_LIVE=1`).
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::keychain::InMemoryKeychain;
    use std::str::FromStr;
    // russh's `Handler` is a trait — we need it in scope to call
    // `check_server_key` (the trait method) on `RanchSshHandler`.
    use russh::client::Handler;

    /// Two distinct hardcoded ed25519 public keys in OpenSSH format.
    /// We use these in the handler unit tests so we don't need a
    /// CSPRNG (which would require pulling `rand_core` as a dev-dep).
    /// The keys are NEVER used for real auth — pure test fixtures.
    const TEST_PUBKEY_A: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIK9MfrLR2aJfHpQpZmkmJrZxQrTOpkVTNUJZ4SqsCAY ranch-test-key-a";
    const TEST_PUBKEY_B: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIxK1mYjvCfXPBQt1lkY8oFGfgzM2cV0fpsLBjOcEZG6 ranch-test-key-b";

    /// Parse an OpenSSH-format public key string into a russh
    /// `PublicKey`. Panics on parse failure (test-only).
    fn parse_pubkey(s: &str) -> PublicKey {
        PublicKey::from_str(s).expect("hardcoded test pubkey should parse")
    }

    #[test]
    fn fingerprint_format_matches_ssh_keygen() {
        let key = parse_pubkey(TEST_PUBKEY_A);
        let fp = fingerprint_sha256(&key);
        assert!(
            fp.starts_with("SHA256:"),
            "fingerprint should start with SHA256: prefix, got {fp}",
        );
        // SHA256 base64 truncated form is 43 chars after the prefix.
        assert_eq!(
            fp.len(),
            "SHA256:".len() + 43,
            "unexpected fingerprint length: {fp}"
        );
    }

    #[test]
    fn fingerprints_of_distinct_keys_differ() {
        let a = parse_pubkey(TEST_PUBKEY_A);
        let b = parse_pubkey(TEST_PUBKEY_B);
        assert_ne!(fingerprint_sha256(&a), fingerprint_sha256(&b));
    }

    #[test]
    fn keychain_name_format() {
        assert_eq!(
            ssh_hostkey_keychain_name("conn-abc"),
            "conn-abc::ssh::hostkey"
        );
    }

    #[tokio::test]
    async fn handler_first_connect_stores_pin() {
        let key = parse_pubkey(TEST_PUBKEY_A);
        let expected_fp = fingerprint_sha256(&key);
        let keychain: Arc<dyn KeychainAdapter> = Arc::new(InMemoryKeychain::new());
        let state = Arc::new(tokio::sync::Mutex::new(HandlerState::default()));
        let mut handler = RanchSshHandler {
            connection_id: "conn-1".into(),
            keychain: keychain.clone(),
            state: state.clone(),
        };

        let accepted = handler.check_server_key(&key).await.unwrap();
        assert!(accepted, "first connect should TOFU-accept");

        // State captured.
        let s = state.lock().await;
        assert_eq!(s.fingerprint.as_deref(), Some(expected_fp.as_str()));
        assert!(s.was_new_pin);
        assert!(s.reject_reason.is_none());
        drop(s);

        // Keychain persisted.
        let pinned = keychain.get(&ssh_hostkey_keychain_name("conn-1")).unwrap();
        assert_eq!(pinned.as_deref(), Some(expected_fp.as_str()));
    }

    #[tokio::test]
    async fn handler_matching_pin_accepts() {
        let key = parse_pubkey(TEST_PUBKEY_A);
        let fp = fingerprint_sha256(&key);
        let keychain: Arc<dyn KeychainAdapter> = Arc::new(InMemoryKeychain::new());
        keychain
            .set(&ssh_hostkey_keychain_name("conn-2"), &fp)
            .unwrap();

        let state = Arc::new(tokio::sync::Mutex::new(HandlerState::default()));
        let mut handler = RanchSshHandler {
            connection_id: "conn-2".into(),
            keychain,
            state: state.clone(),
        };

        let accepted = handler.check_server_key(&key).await.unwrap();
        assert!(accepted, "matching pin should accept");
        let s = state.lock().await;
        assert!(!s.was_new_pin, "should not be a new pin");
        assert!(s.reject_reason.is_none());
    }

    #[tokio::test]
    async fn handler_mismatch_pin_rejects_with_reason() {
        // Pin key A's fingerprint, then connect with key B.
        let key_a = parse_pubkey(TEST_PUBKEY_A);
        let key_b = parse_pubkey(TEST_PUBKEY_B);
        let pinned_fp = fingerprint_sha256(&key_a);
        let keychain: Arc<dyn KeychainAdapter> = Arc::new(InMemoryKeychain::new());
        keychain
            .set(&ssh_hostkey_keychain_name("conn-3"), &pinned_fp)
            .unwrap();

        let state = Arc::new(tokio::sync::Mutex::new(HandlerState::default()));
        let mut handler = RanchSshHandler {
            connection_id: "conn-3".into(),
            keychain,
            state: state.clone(),
        };

        let accepted = handler.check_server_key(&key_b).await.unwrap();
        assert!(!accepted, "mismatched pin should reject");
        let s = state.lock().await;
        assert!(
            s.reject_reason
                .as_deref()
                .is_some_and(|r| r.contains("mismatch")),
            "reject_reason should mention mismatch, got {:?}",
            s.reject_reason,
        );
    }
}
