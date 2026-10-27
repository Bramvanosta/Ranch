//! Integration tests for SSH tunnel data layer.
//!
//! Three concerns covered:
//!   1. Persistence round-trip (key-file + password variants, with/without
//!      jump host) and JSON-corruption forgiveness on read.
//!   2. Update / clear / orphan-cleanup semantics for the SSH config and
//!      its keychain entry at `{id}::ssh`.
//!   3. `test_connection` and `ping_connection` placeholder when SSH is
//!      enabled (no socket open, deferred-runtime reason surfaced).
//!   4. `remove_connection` and `duplicate_connection` interactions with
//!      the SSH keychain entry.
//!
//! No live `russh` session here — see `ssh_tunnel_runtime.rs`.

use ranch_lib::commands::connections::{
    add_connection_inner, duplicate_connection_inner, list_connections_inner,
    ping_connection_inner, remove_connection_inner, test_connection_inner, update_connection_inner,
    AddConnectionArgs, TestConnectionArgs, TestConnectionError, UpdateConnectionArgs,
};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::core::db::Db;
use ranch_lib::core::errors::RanchError;
use ranch_lib::core::keychain::InMemoryKeychain;
use ranch_lib::core::ssh_config::{SshAuthMethod, SshJumpHost, SshTunnelConfig};
use tempfile::tempdir;

fn key_file_cfg() -> SshTunnelConfig {
    SshTunnelConfig {
        host: "bastion.example.com".into(),
        port: 22,
        user: "ubuntu".into(),
        auth: SshAuthMethod::KeyFile {
            key_file_path: "/Users/alex/.ssh/id_ed25519".into(),
        },
        jump_host: None,
    }
}

fn password_cfg() -> SshTunnelConfig {
    SshTunnelConfig {
        host: "bastion.example.com".into(),
        port: 22,
        user: "ubuntu".into(),
        auth: SshAuthMethod::Password,
        jump_host: None,
    }
}

fn add_args(
    name: &str,
    ssh: Option<SshTunnelConfig>,
    ssh_password: Option<&str>,
) -> AddConnectionArgs {
    AddConnectionArgs {
        locked: None,
        name: name.to_string(),
        host: "h.example.com".to_string(),
        port: 6379,
        db: 0,
        username: None,
        password: None,
        tls: false,
        tls_custom_ca: None,
        ssh_tunnel: ssh,
        ssh_password: ssh_password.map(str::to_string),
        group: GroupSelector::named("Default"),
        environment: EnvironmentSelector::for_color("env-local"),
    }
}

fn update_args_from(conn: &ranch_lib::commands::connections::Connection) -> UpdateConnectionArgs {
    UpdateConnectionArgs {
        locked: None,
        id: conn.id.clone(),
        name: conn.name.clone(),
        host: conn.host.clone(),
        port: conn.port,
        db: conn.db,
        username: conn.username.clone(),
        password: None,
        tls: conn.tls,
        tls_custom_ca: None,
        ssh_tunnel: None,
        ssh_tunnel_clear: false,
        ssh_password: None,
        group: GroupSelector::Existing {
            id: conn.group_id.clone(),
        },
        environment: EnvironmentSelector::Existing {
            id: conn.environment_id.clone(),
        },
    }
}

// ── 1. Persistence round-trip ────────────────────────────────────────────

#[tokio::test]
async fn add_with_key_file_ssh_persists_and_lists() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let cfg = key_file_cfg();
    let conn = add_connection_inner(add_args("kf", Some(cfg.clone()), None), db.pool(), &kc)
        .await
        .unwrap();
    assert_eq!(conn.ssh_tunnel.as_ref().unwrap(), &cfg);

    let listed = list_connections_inner(db.pool()).await.unwrap();
    assert_eq!(listed[0].ssh_tunnel.as_ref().unwrap(), &cfg);
}

#[tokio::test]
async fn add_with_password_ssh_persists_config_and_writes_keychain() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        add_args("pw", Some(password_cfg()), Some("ssh-secret")),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();

    // Config persisted (without password).
    let listed = list_connections_inner(db.pool()).await.unwrap();
    assert!(matches!(
        listed[0].ssh_tunnel.as_ref().unwrap().auth,
        SshAuthMethod::Password
    ));

    // SSH password lives at `{id}::ssh`, NOT at `{id}` (which would collide
    // with the Redis password).
    let ssh_kc_name = format!("{}::ssh", conn.id);
    assert_eq!(kc.get(&ssh_kc_name).unwrap().as_deref(), Some("ssh-secret"));
    // Redis password keychain entry is empty (no Redis password was set).
    assert!(kc.get(&conn.id).unwrap().is_none());
}

#[tokio::test]
async fn add_with_password_ssh_but_no_password_skips_keychain_write() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        add_args("pw-no-secret", Some(password_cfg()), None),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();

    let ssh_kc_name = format!("{}::ssh", conn.id);
    assert!(kc.get(&ssh_kc_name).unwrap().is_none());
}

#[tokio::test]
async fn add_rejects_invalid_ssh_config() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let mut bad = key_file_cfg();
    bad.host = "".into();
    let err = add_connection_inner(add_args("bad", Some(bad), None), db.pool(), &kc)
        .await
        .unwrap_err();
    assert!(matches!(err, RanchError::InvalidInput { .. }));
}

#[tokio::test]
async fn add_with_jump_host_round_trips() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let mut cfg = key_file_cfg();
    cfg.jump_host = Some(SshJumpHost {
        host: "jump.example.com".into(),
        port: 22,
        user: "jumper".into(),
    });
    let conn = add_connection_inner(add_args("jh", Some(cfg.clone()), None), db.pool(), &kc)
        .await
        .unwrap();
    assert_eq!(conn.ssh_tunnel.as_ref().unwrap(), &cfg);

    let listed = list_connections_inner(db.pool()).await.unwrap();
    assert_eq!(
        listed[0].ssh_tunnel.as_ref().unwrap().jump_host,
        cfg.jump_host
    );
}

// ── 2. Update / clear / orphan cleanup ───────────────────────────────────

#[tokio::test]
async fn update_can_set_ssh_config_on_a_previously_direct_connection() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(add_args("c", None, None), db.pool(), &kc)
        .await
        .unwrap();
    assert!(conn.ssh_tunnel.is_none());

    let updated = update_connection_inner(
        UpdateConnectionArgs {
            locked: None,
            ssh_tunnel: Some(key_file_cfg()),
            ..update_args_from(&conn)
        },
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    assert!(updated.ssh_tunnel.is_some());
}

#[tokio::test]
async fn update_clear_removes_config_and_keychain_entry() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        add_args("pw", Some(password_cfg()), Some("secret")),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    let ssh_kc_name = format!("{}::ssh", conn.id);
    assert_eq!(kc.get(&ssh_kc_name).unwrap().as_deref(), Some("secret"));

    let updated = update_connection_inner(
        UpdateConnectionArgs {
            locked: None,
            ssh_tunnel_clear: true,
            ..update_args_from(&conn)
        },
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    assert!(updated.ssh_tunnel.is_none());
    assert!(
        kc.get(&ssh_kc_name).unwrap().is_none(),
        "keychain entry should be wiped"
    );
}

#[tokio::test]
async fn update_switching_password_to_keyfile_deletes_orphan_keychain_entry() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        add_args("c", Some(password_cfg()), Some("orphan-secret")),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    let ssh_kc_name = format!("{}::ssh", conn.id);
    assert_eq!(
        kc.get(&ssh_kc_name).unwrap().as_deref(),
        Some("orphan-secret")
    );

    // Switch to key-file auth — the password keychain entry must be cleaned up.
    let _ = update_connection_inner(
        UpdateConnectionArgs {
            locked: None,
            ssh_tunnel: Some(key_file_cfg()),
            ..update_args_from(&conn)
        },
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    assert!(
        kc.get(&ssh_kc_name).unwrap().is_none(),
        "keychain entry should be cleaned up on auth-method switch"
    );
}

#[tokio::test]
async fn update_rejects_empty_ssh_password() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(add_args("c", None, None), db.pool(), &kc)
        .await
        .unwrap();
    let err = update_connection_inner(
        UpdateConnectionArgs {
            locked: None,
            ssh_password: Some("".to_string()),
            ..update_args_from(&conn)
        },
        db.pool(),
        &kc,
    )
    .await
    .unwrap_err();
    match err {
        RanchError::InvalidInput { reason } => {
            assert!(reason.contains("SSH password"), "reason: {reason}");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[tokio::test]
async fn update_can_rotate_ssh_password_independently() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        add_args("c", Some(password_cfg()), Some("v1")),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();

    // Update with ssh_tunnel: None (leave config alone) but ssh_password: Some("v2").
    let _ = update_connection_inner(
        UpdateConnectionArgs {
            locked: None,
            ssh_password: Some("v2".to_string()),
            ..update_args_from(&conn)
        },
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    let ssh_kc_name = format!("{}::ssh", conn.id);
    assert_eq!(kc.get(&ssh_kc_name).unwrap().as_deref(), Some("v2"));
}

// ── 3. test_connection now actually attempts the SSH tunnel ──

#[tokio::test]
async fn test_connection_attempts_real_ssh_tunnel() {
    // With SSH configured the call actually tries to open the tunnel. With an
    // unreachable bastion (127.0.0.1:1 refuses) we should get a typed
    // SshTunnel error, NOT a placeholder.
    let mut cfg = key_file_cfg();
    cfg.host = "127.0.0.1".into();
    cfg.port = 1; // refused-fast
    cfg.auth = ranch_lib::core::ssh_config::SshAuthMethod::KeyFile {
        key_file_path: "/tmp/this-key-does-not-exist".into(),
    };
    let r = test_connection_inner(TestConnectionArgs {
        host: "127.0.0.1".into(),
        port: 6379,
        db: 0,
        username: None,
        password: None,
        tls: false,
        tls_custom_ca: None,
        ssh_tunnel: Some(cfg),
        ssh_password: None,
        saved_connection_id: None,
    })
    .await
    .unwrap();
    assert!(!r.success);
    match r.error.unwrap() {
        // The key file doesn't exist → SshTunnel error variant.
        TestConnectionError::SshTunnel { reason } => {
            assert!(!reason.is_empty(), "the SSH error says why; got: {reason}");
        }
        other => panic!("expected SshTunnel, got {other:?}"),
    }
}

#[tokio::test]
async fn ping_connection_attempts_real_ssh_tunnel() {
    // `ping_connection` actually attempts the SSH tunnel. With a
    // missing key file we should see a typed SshTunnel error.
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let mut cfg = key_file_cfg();
    cfg.auth = ranch_lib::core::ssh_config::SshAuthMethod::KeyFile {
        key_file_path: "/tmp/this-key-does-not-exist".into(),
    };
    let conn = add_connection_inner(add_args("c", Some(cfg), None), db.pool(), &kc)
        .await
        .unwrap();

    let r = ping_connection_inner(&conn.id, db.pool(), &kc, None)
        .await
        .unwrap();
    assert!(!r.success);
    match r.error.unwrap() {
        TestConnectionError::SshTunnel { reason } => {
            assert!(!reason.is_empty(), "the SSH error says why; got: {reason}");
        }
        other => panic!("expected SshTunnel, got {other:?}"),
    }
}

// ── 4. Remove + duplicate interactions ───────────────────────────────────

#[tokio::test]
async fn remove_cleans_up_ssh_keychain_entry() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        add_args("c", Some(password_cfg()), Some("ssh-secret")),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();
    let ssh_kc_name = format!("{}::ssh", conn.id);
    assert!(kc.get(&ssh_kc_name).unwrap().is_some());

    remove_connection_inner(&conn.id, db.pool(), &kc, None)
        .await
        .unwrap();
    assert!(
        kc.get(&ssh_kc_name).unwrap().is_none(),
        "SSH keychain entry should be cleaned up"
    );
}

#[tokio::test]
async fn duplicate_carries_ssh_config_and_password_independently() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(
        add_args("orig", Some(password_cfg()), Some("ssh-secret")),
        db.pool(),
        &kc,
    )
    .await
    .unwrap();

    let dup = duplicate_connection_inner(&conn.id, db.pool(), &kc)
        .await
        .unwrap();
    assert_ne!(dup.id, conn.id);
    assert_eq!(dup.ssh_tunnel, conn.ssh_tunnel);

    // Independent keychain entry.
    let dup_ssh_kc = format!("{}::ssh", dup.id);
    assert_eq!(kc.get(&dup_ssh_kc).unwrap().as_deref(), Some("ssh-secret"));

    // Independence: rotating the source's secret doesn't affect the duplicate.
    let src_ssh_kc = format!("{}::ssh", conn.id);
    kc.set(&src_ssh_kc, "rotated").unwrap();
    assert_eq!(kc.get(&dup_ssh_kc).unwrap().as_deref(), Some("ssh-secret"));
}

#[tokio::test]
async fn duplicate_without_ssh_password_does_not_create_orphan_entry() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    let conn = add_connection_inner(add_args("orig", Some(key_file_cfg()), None), db.pool(), &kc)
        .await
        .unwrap();
    let dup = duplicate_connection_inner(&conn.id, db.pool(), &kc)
        .await
        .unwrap();
    let dup_ssh_kc = format!("{}::ssh", dup.id);
    assert!(kc.get(&dup_ssh_kc).unwrap().is_none());
    assert!(dup.ssh_tunnel.is_some(), "config still copied");
}

// ── 5. Defensive read on corrupt JSON ───────────────────────────────────

#[tokio::test]
async fn list_connections_returns_none_for_corrupt_ssh_tunnel_json() {
    let dir = tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: std::sync::Arc<dyn ranch_lib::core::keychain::KeychainAdapter> =
        std::sync::Arc::new(InMemoryKeychain::new());

    // Insert a connection with valid SSH config first.
    let conn = add_connection_inner(add_args("c", Some(key_file_cfg()), None), db.pool(), &kc)
        .await
        .unwrap();

    // Corrupt the column directly via SQL.
    sqlx::query("UPDATE connections SET ssh_tunnel_config = ? WHERE id = ?")
        .bind("not valid json {{{")
        .bind(&conn.id)
        .execute(db.pool())
        .await
        .unwrap();

    // List should still succeed; the corrupt row's ssh_tunnel resolves to None.
    let listed = list_connections_inner(db.pool()).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(
        listed[0].ssh_tunnel.is_none(),
        "corrupt JSON should decode to None, not crash"
    );
}
