//! SSH tunnel configuration types.
//!
//! Persistent shape: JSON, the `sshTunnel` of a BullMQ-on-Redis
//! connection's endpoint; `contract::connections` mirrors it for the
//! frontend.
//!
//! Runtime tunneling via `russh` lives in `ranch_bullmq_redis::ssh_tunnel`
//! This module is pure data: serialize, deserialize, validate.
//!
//! Key design decisions:
//!   - The boundary type [`SshTunnelConfig`] does NOT carry the SSH
//!     password. The frontend learns "a password exists" from
//!     `auth.kind == "password"` and renders the
//!     `Leave blank to keep current` placeholder accordingly. The actual
//!     SSH password lives in the OS keychain at `{connection_id}::ssh`
//!     (the Redis password is stored at `{connection_id}`; the SSH
//!     password uses the same adapter with a different name).
//!   - Stored as JSON inside the endpoint, NOT as a normalized set of
//!     columns: only the BullMQ backend reads it.

use serde::{Deserialize, Serialize};

use crate::errors::RanchError;

// Serialize/Deserialize: stored as JSON in a connection's endpoint.
/// The structural SSH tunnel config
/// (host / port / user / auth method / optional jump host): no password.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SshTunnelConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuthMethod,
    pub jump_host: Option<SshJumpHost>,
}

/// Discriminated auth-method union.
///
/// `KeyFile` carries a path string; we do NOT read or validate the file
/// contents here (`ranch_bullmq_redis::ssh_tunnel` does at connect time).
/// `Password` is a marker variant: the actual password text lives in the
/// keychain at `{connection_id}::ssh`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SshAuthMethod {
    KeyFile { key_file_path: String },
    Password,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SshJumpHost {
    pub host: String,
    pub port: u16,
    pub user: String,
}

/// Structural validation. Run this BEFORE persisting or before passing the
/// config to `test_connection`.
///
/// Empty host / user / key-file-path → `InvalidInput`. Port 0 → `InvalidInput`.
/// Jump host fields, if present, validated identically.
pub fn validate_ssh_tunnel_config(cfg: &SshTunnelConfig) -> Result<(), RanchError> {
    if cfg.host.trim().is_empty() {
        return Err(RanchError::InvalidInput {
            reason: "SSH host is required".into(),
        });
    }
    if cfg.user.trim().is_empty() {
        return Err(RanchError::InvalidInput {
            reason: "SSH user is required".into(),
        });
    }
    if cfg.port == 0 {
        return Err(RanchError::InvalidInput {
            reason: "SSH port must be 1..65535".into(),
        });
    }
    if let SshAuthMethod::KeyFile { key_file_path } = &cfg.auth {
        if key_file_path.trim().is_empty() {
            return Err(RanchError::InvalidInput {
                reason: "SSH key file path is required when auth = keyFile".into(),
            });
        }
    }
    if let Some(jh) = &cfg.jump_host {
        if jh.host.trim().is_empty() || jh.user.trim().is_empty() || jh.port == 0 {
            return Err(RanchError::InvalidInput {
                reason: "SSH jump host requires host, user, and a non-zero port".into(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_to_json(cfg: &SshTunnelConfig) -> serde_json::Result<String> {
        serde_json::to_string(cfg)
    }

    fn decode_from_json(s: &str) -> serde_json::Result<SshTunnelConfig> {
        serde_json::from_str(s)
    }

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

    #[test]
    fn round_trip_key_file_variant() {
        let cfg = key_file_cfg();
        let json = encode_to_json(&cfg).unwrap();
        let back = decode_from_json(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn round_trip_password_variant() {
        let cfg = password_cfg();
        let json = encode_to_json(&cfg).unwrap();
        let back = decode_from_json(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn round_trip_with_jump_host() {
        let mut cfg = key_file_cfg();
        cfg.jump_host = Some(SshJumpHost {
            host: "jump.example.com".into(),
            port: 22,
            user: "jumpuser".into(),
        });
        let json = encode_to_json(&cfg).unwrap();
        let back = decode_from_json(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn json_does_not_contain_a_password_field() {
        // Defensive: this struct has no `password` *field*. If a future
        // refactor accidentally adds one (e.g. `pub password: String`),
        // this test catches the leak. We allow the literal "password" to
        // appear as the auth-kind discriminator value (`"kind":"password"`)
        // That's the marker variant, not a credential.
        let cfg = password_cfg();
        let json = encode_to_json(&cfg).unwrap();
        assert!(
            !json.contains("\"password\":"),
            "JSON must not contain a `password` FIELD (only the auth-kind value); got: {json}"
        );
    }

    #[test]
    fn validate_rejects_empty_host() {
        let mut cfg = key_file_cfg();
        cfg.host = "".into();
        let err = validate_ssh_tunnel_config(&cfg).unwrap_err();
        assert!(matches!(err, RanchError::InvalidInput { ref reason } if reason.contains("host")));
    }

    #[test]
    fn validate_rejects_empty_user() {
        let mut cfg = key_file_cfg();
        cfg.user = "  ".into();
        let err = validate_ssh_tunnel_config(&cfg).unwrap_err();
        assert!(matches!(err, RanchError::InvalidInput { ref reason } if reason.contains("user")));
    }

    #[test]
    fn validate_rejects_zero_port() {
        let mut cfg = key_file_cfg();
        cfg.port = 0;
        let err = validate_ssh_tunnel_config(&cfg).unwrap_err();
        assert!(matches!(err, RanchError::InvalidInput { ref reason } if reason.contains("port")));
    }

    #[test]
    fn validate_rejects_empty_key_file_path() {
        let cfg = SshTunnelConfig {
            host: "h".into(),
            port: 22,
            user: "u".into(),
            auth: SshAuthMethod::KeyFile {
                key_file_path: "".into(),
            },
            jump_host: None,
        };
        let err = validate_ssh_tunnel_config(&cfg).unwrap_err();
        assert!(
            matches!(err, RanchError::InvalidInput { ref reason } if reason.contains("key file"))
        );
    }

    #[test]
    fn validate_password_variant_does_not_require_key_file_path() {
        let cfg = password_cfg();
        validate_ssh_tunnel_config(&cfg).unwrap();
    }

    #[test]
    fn validate_rejects_jump_host_with_empty_fields() {
        let mut cfg = key_file_cfg();
        cfg.jump_host = Some(SshJumpHost {
            host: "".into(),
            port: 22,
            user: "u".into(),
        });
        let err = validate_ssh_tunnel_config(&cfg).unwrap_err();
        assert!(matches!(err, RanchError::InvalidInput { ref reason } if reason.contains("jump")));
    }

    #[test]
    fn decode_garbage_is_rejected() {
        assert!(decode_from_json("not json at all").is_err());
    }
}
