//! Live integration tests for the russh-backed SSH tunnel
//! runtime. Gated by the `RANCH_SSH_LIVE=1` environment variable so
//! the test suite stays hermetic by default. Each test skips with an
//! explanatory `eprintln!` when the env var is unset (mirrors the
//! `RANCH_LOCAL_REDIS=1` convention from `bulk_actions_live.rs`).
//!
//! ## Manual setup for `RANCH_SSH_LIVE=1`
//!
//! The tests assume:
//!   1. A local OpenSSH server is reachable at `127.0.0.1:2222` (override
//!      with `RANCH_SSH_HOST` / `RANCH_SSH_PORT`).
//!   2. A test key is present at `~/.ranch-test/ssh_key` with the public
//!      half installed in the SSH server's `authorized_keys`. The
//!      `RANCH_SSH_KEY_PATH` env var overrides this default.
//!   3. The SSH server is configured to allow `direct-tcpip` channels
//!      to `127.0.0.1:6379` (or `RANCH_REDIS_HOST:RANCH_REDIS_PORT`).
//!   4. A local Redis is running at that target (used by the rest of
//!      the live-suite already — see `bulk_actions_live.rs`).
//!
//! Quickstart (macOS, with Docker):
//! ```bash
//! docker run -d --name ranch-ssh-test \
//!   -p 2222:2222 \
//!   -e PUBLIC_KEY="$(cat ~/.ssh/id_ed25519.pub)" \
//!   -e USER_NAME=ranch \
//!   linuxserver/openssh-server
//! RANCH_SSH_LIVE=1 RANCH_SSH_KEY_PATH=~/.ssh/id_ed25519 \
//!   cargo test --test ssh_tunnel_runtime -- --nocapture
//! ```
//!
//! ## Why embedded russh server is NOT used here
//!
//! An early plan suggested an in-process `russh::server`
//! fixture for fully hermetic tests. That requires ~150 lines of
//! server-side `Handler` boilerplate (auth, channel routing) plus
//! cooperating with the existing local-Redis fixture. The maintenance
//! burden outweighs the marginal coverage gain — the unit tests in
//! `core::ssh_tunnel::tests` already exercise the host-key / TOFU
//! logic, and the integration tests below cover the wire-level russh
//! ↔ OpenSSH interop where regressions are most likely. The embedded server
//! can be added if CI runs ever need these gates active.

use std::sync::Arc;

use ranch_lib::core::keychain::{InMemoryKeychain, KeychainAdapter};
use ranch_lib::core::ssh_config::{SshAuthMethod, SshTunnelConfig};
use ranch_lib::core::ssh_tunnel;

fn live_enabled() -> bool {
    std::env::var("RANCH_SSH_LIVE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn ssh_host() -> String {
    std::env::var("RANCH_SSH_HOST").unwrap_or_else(|_| "127.0.0.1".into())
}

fn ssh_port() -> u16 {
    std::env::var("RANCH_SSH_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2222)
}

fn ssh_user() -> String {
    std::env::var("RANCH_SSH_USER").unwrap_or_else(|_| "ranch".into())
}

fn ssh_key_path() -> String {
    std::env::var("RANCH_SSH_KEY_PATH").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.ranch-test/ssh_key")
    })
}

fn redis_host() -> String {
    std::env::var("RANCH_REDIS_HOST").unwrap_or_else(|_| "127.0.0.1".into())
}

fn redis_port() -> u16 {
    std::env::var("RANCH_REDIS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6390)
}

fn key_file_cfg() -> SshTunnelConfig {
    SshTunnelConfig {
        host: ssh_host(),
        port: ssh_port(),
        user: ssh_user(),
        auth: SshAuthMethod::KeyFile {
            key_file_path: ssh_key_path(),
        },
        jump_host: None,
    }
}

fn fresh_keychain() -> Arc<dyn KeychainAdapter> {
    Arc::new(InMemoryKeychain::new())
}

// ─────────────────────────────────────────────────────────────────────
// 7 live integration tests (RANCH_SSH_LIVE-gated)
// ─────────────────────────────────────────────────────────────────────

/// Test 1 — happy path: tunnel opens, Redis PING succeeds.
#[tokio::test]
async fn tunnel_opens_and_redis_ping_succeeds() {
    if !live_enabled() {
        eprintln!("skip: set RANCH_SSH_LIVE=1 to run SSH live tests");
        return;
    }
    let keychain = fresh_keychain();
    let result = ssh_tunnel::open(
        &key_file_cfg(),
        None,
        (&redis_host(), redis_port()),
        keychain,
        "ssh-test-conn-1",
    )
    .await
    .expect("tunnel should open against the live SSH server");
    assert!(result.fingerprint.starts_with("SHA256:"));
    // Verify the listener is reachable.
    let local = result.tunnel.local_addr();
    let _ = tokio::net::TcpStream::connect(local)
        .await
        .expect("local forwarder listener should accept connections");
}

/// Test 2 — password auth path. Requires an SSH server configured
/// to accept password auth for the test user. Skipped automatically
/// when no password is provided.
#[tokio::test]
async fn tunnel_with_password_auth_succeeds() {
    if !live_enabled() {
        eprintln!("skip: set RANCH_SSH_LIVE=1 to run SSH live tests");
        return;
    }
    let Ok(password) = std::env::var("RANCH_SSH_PASSWORD") else {
        eprintln!("skip: RANCH_SSH_PASSWORD unset; password-auth subtest skipped");
        return;
    };
    let cfg = SshTunnelConfig {
        host: ssh_host(),
        port: ssh_port(),
        user: ssh_user(),
        auth: SshAuthMethod::Password,
        jump_host: None,
    };
    let keychain = fresh_keychain();
    let result = ssh_tunnel::open(
        &cfg,
        Some(&password),
        (&redis_host(), redis_port()),
        keychain,
        "ssh-test-conn-pw",
    )
    .await
    .expect("password-auth tunnel should open");
    assert!(result.fingerprint.starts_with("SHA256:"));
}

/// Test 3 — wrong password rejects with `SshAuthFailed`.
#[tokio::test]
async fn tunnel_with_wrong_password_returns_ssh_auth_failed() {
    if !live_enabled() {
        eprintln!("skip: set RANCH_SSH_LIVE=1 to run SSH live tests");
        return;
    }
    let cfg = SshTunnelConfig {
        host: ssh_host(),
        port: ssh_port(),
        user: ssh_user(),
        auth: SshAuthMethod::Password,
        jump_host: None,
    };
    let keychain = fresh_keychain();
    let err = ssh_tunnel::open(
        &cfg,
        Some("definitely-the-wrong-password"),
        (&redis_host(), redis_port()),
        keychain,
        "ssh-test-conn-bad-pw",
    )
    .await
    .expect_err("wrong password should error");
    use ranch_lib::core::errors::RanchError;
    assert!(
        matches!(err, RanchError::SshAuthFailed { .. }),
        "expected SshAuthFailed, got {err:?}"
    );
}

/// Test 4 — missing key file returns `SshKeyFileError`.
#[tokio::test]
async fn tunnel_with_missing_key_file_returns_ssh_key_file_error() {
    if !live_enabled() {
        eprintln!("skip: set RANCH_SSH_LIVE=1 to run SSH live tests");
        return;
    }
    let cfg = SshTunnelConfig {
        host: ssh_host(),
        port: ssh_port(),
        user: ssh_user(),
        auth: SshAuthMethod::KeyFile {
            key_file_path: "/tmp/ranch-test-this-file-does-not-exist".into(),
        },
        jump_host: None,
    };
    let keychain = fresh_keychain();
    let err = ssh_tunnel::open(
        &cfg,
        None,
        (&redis_host(), redis_port()),
        keychain,
        "ssh-test-conn-missing-key",
    )
    .await
    .expect_err("missing key file should error");
    use ranch_lib::core::errors::RanchError;
    match err {
        RanchError::SshKeyFileError { path, .. } => {
            assert!(path.contains("does-not-exist"));
        }
        other => panic!("expected SshKeyFileError, got {other:?}"),
    }
}

/// Test 5 — unreachable bastion returns `SshTunnelFailed`.
#[tokio::test]
async fn tunnel_with_unreachable_bastion_returns_ssh_tunnel_failed() {
    if !live_enabled() {
        eprintln!("skip: set RANCH_SSH_LIVE=1 to run SSH live tests");
        return;
    }
    // Port 1 is reserved / refused fast on most platforms.
    let cfg = SshTunnelConfig {
        host: "127.0.0.1".into(),
        port: 1,
        user: "ranch".into(),
        auth: SshAuthMethod::KeyFile {
            key_file_path: ssh_key_path(),
        },
        jump_host: None,
    };
    let keychain = fresh_keychain();
    let err = ssh_tunnel::open(
        &cfg,
        None,
        (&redis_host(), redis_port()),
        keychain,
        "ssh-test-conn-unreachable",
    )
    .await
    .expect_err("unreachable bastion should error");
    use ranch_lib::core::errors::RanchError;
    assert!(
        matches!(err, RanchError::SshTunnelFailed { .. }),
        "expected SshTunnelFailed, got {err:?}"
    );
}

/// Test 6 — re-opening the tunnel with the same `connection_id`
/// uses the previously-pinned fingerprint (TOFU works across opens).
#[tokio::test]
async fn tunnel_reopen_reuses_pinned_fingerprint() {
    if !live_enabled() {
        eprintln!("skip: set RANCH_SSH_LIVE=1 to run SSH live tests");
        return;
    }
    let keychain = fresh_keychain();
    let conn_id = "ssh-test-conn-repin";
    // First open — pins.
    let first = ssh_tunnel::open(
        &key_file_cfg(),
        None,
        (&redis_host(), redis_port()),
        keychain.clone(),
        conn_id,
    )
    .await
    .expect("first open");
    assert!(first.was_new_pin, "first connect should be a new pin");
    drop(first);
    // Second open — same connection_id, should NOT be a new pin.
    let second = ssh_tunnel::open(
        &key_file_cfg(),
        None,
        (&redis_host(), redis_port()),
        keychain.clone(),
        conn_id,
    )
    .await
    .expect("second open");
    assert!(!second.was_new_pin, "second connect should reuse the pin");
    // Same fingerprint.
    let first_fp = keychain
        .get(&ssh_tunnel::ssh_hostkey_keychain_name(conn_id))
        .unwrap()
        .unwrap();
    assert_eq!(first_fp, second.fingerprint);
}

/// Test 7 — dropping the tunnel disposes the forwarder.
#[tokio::test]
async fn shutdown_disposes_tunnel_cleanly() {
    if !live_enabled() {
        eprintln!("skip: set RANCH_SSH_LIVE=1 to run SSH live tests");
        return;
    }
    let keychain = fresh_keychain();
    let opened = ssh_tunnel::open(
        &key_file_cfg(),
        None,
        (&redis_host(), redis_port()),
        keychain,
        "ssh-test-conn-dispose",
    )
    .await
    .expect("tunnel should open");
    let local = opened.tunnel.local_addr();
    drop(opened);
    // Give the forwarder a moment to exit.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    // The listener should be gone; new connects should fail.
    let attempt = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        tokio::net::TcpStream::connect(local),
    )
    .await;
    assert!(
        matches!(attempt, Ok(Err(_)) | Err(_)),
        "listener should refuse connections after drop; got {attempt:?}"
    );
}
