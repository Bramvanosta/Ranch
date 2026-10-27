//! TLS helpers shared by every Redis-touching command.
//!
//! Single source of truth for how Ranch:
//!   1. validates a user-supplied CA PEM (`validate_ca_pem`), and
//!   2. constructs a `redis::Client` honoring TLS settings (`build_redis_client`).
//!
//! After this module lands, no other file should call `redis::Client::open`
//! directly. Every Redis-touching command path (`test_connection`,
//! `add_connection`, `update_connection`, `duplicate_connection`,
//! `ping_connection`) goes through `build_redis_client` so TLS behavior is
//! identical across paths.
//!
//! Architectural notes:
//!   - rustls (not OpenSSL) — matches the project-wide pure-Rust TLS choice
//!     baked into `redis = { features = ["tokio-rustls-comp"] }` and sqlx's
//!     `runtime-tokio-rustls`.
//!   - Custom CA is **additive** to system roots, not a replacement. This
//!     matches the AWS Elasticache / corporate-CA expectation: the user pins
//!     a private CA on top of the public store.
//!   - `tls_custom_ca` is a public certificate (NOT a secret) — stays in
//!     SQLite, never touches the keychain.

use std::time::Duration;

use redis::IntoConnectionInfo;

use crate::core::errors::RanchError;

/// How long one Redis reply may take. Sized for a slow link (up
/// to ~1 s round trip) and big pipelines, not redis-rs's
/// 500 ms default, which turned a far Redis into "unreachable".
pub const REDIS_RESPONSE_TIMEOUT: Duration = Duration::from_secs(20);
/// How long opening a connection may take: TCP, TLS and the
/// setup round trip, several round trips on a slow link (redis-rs: 1 s).
pub const REDIS_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeouts for a one-off connection (test, probe).
pub fn async_connection_config() -> redis::AsyncConnectionConfig {
    redis::AsyncConnectionConfig::new()
        .set_response_timeout(Some(REDIS_RESPONSE_TIMEOUT))
        .set_connection_timeout(Some(REDIS_CONNECT_TIMEOUT))
}

/// Timeouts and reconnection for a connection actor's persistent connection.
pub fn connection_manager_config() -> redis::aio::ConnectionManagerConfig {
    redis::aio::ConnectionManagerConfig::new()
        .set_response_timeout(Some(REDIS_RESPONSE_TIMEOUT))
        .set_connection_timeout(Some(REDIS_CONNECT_TIMEOUT))
        .set_number_of_retries(2)
        .set_max_delay(Duration::from_secs(2))
}

/// Validate a user-supplied CA PEM blob.
///
/// Semantics:
///   - `None` or `Some("")` or whitespace-only → returns `Ok(None)` (treated
///     as "no custom CA"; falls back to system trust roots).
///   - PEM containing ≥1 `-----BEGIN CERTIFICATE-----` block AND no other
///     PEM types → returns `Ok(Some(trimmed_pem))`.
///   - Unparseable input → `Err(InvalidInput { reason: "Invalid CA PEM: …" })`.
///   - PEM containing non-CERTIFICATE blocks (private keys, CSRs, etc.) →
///     `Err(InvalidInput)` — only CERTIFICATE blocks are accepted because the
///     intended use is "trust root", not "client identity" (mTLS is out of
///     scope).
pub fn validate_ca_pem(input: Option<&str>) -> Result<Option<String>, RanchError> {
    let Some(raw) = input else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let mut cursor = std::io::Cursor::new(trimmed.as_bytes());
    let items: Vec<_> = rustls_pemfile::read_all(&mut cursor)
        .collect::<Result<_, _>>()
        .map_err(|e| RanchError::InvalidInput {
            reason: format!("Invalid CA PEM: {e}"),
        })?;

    let cert_count = items
        .iter()
        .filter(|i| matches!(i, rustls_pemfile::Item::X509Certificate(_)))
        .count();
    if cert_count == 0 {
        return Err(RanchError::InvalidInput {
            reason: "Invalid CA PEM: no -----BEGIN CERTIFICATE----- block found".to_string(),
        });
    }

    // Reject mixed blocks (private keys etc.) — only CERTIFICATE allowed.
    if items
        .iter()
        .any(|i| !matches!(i, rustls_pemfile::Item::X509Certificate(_)))
    {
        return Err(RanchError::InvalidInput {
            reason: "Invalid CA PEM: only CERTIFICATE blocks are accepted (no keys/CSRs)"
                .to_string(),
        });
    }

    Ok(Some(trimmed.to_string()))
}

/// Build a `redis::Client` honoring TLS settings.
///
/// `tls_custom_ca` MUST be the already-validated PEM (caller ran
/// [`validate_ca_pem`] first). When TLS is OFF or no custom CA is provided,
/// falls through to the cheaper URL-based path that uses system trust roots.
///
/// When a custom CA is provided, calls `redis::Client::build_with_tls`
/// passing the PEM bytes via `redis::TlsCertificates`. The certificate is
/// added on top of system roots — both system-signed certs (e.g. Upstash)
/// and the user-pinned CA (e.g. private Elasticache) validate successfully.
///
/// Nagle is off (`TCP_NODELAY`): Ranch sends small commands and
/// waits for each reply, the case where Nagle adds delay.
pub fn build_redis_client(
    url: &str,
    tls_custom_ca: Option<&str>,
) -> Result<redis::Client, redis::RedisError> {
    let info = url
        .into_connection_info()?
        .set_tcp_settings(redis::io::tcp::TcpSettings::default().set_nodelay(true));
    match tls_custom_ca {
        Some(pem) if !pem.trim().is_empty() => redis::Client::build_with_tls(
            info,
            redis::TlsCertificates {
                client_tls: None,
                root_cert: Some(pem.as_bytes().to_vec()),
            },
        ),
        _ => redis::Client::open(info),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_CA_PEM: &str = include_str!("../../tests/fixtures/self_signed_ca.pem");

    #[test]
    fn empty_input_is_none() {
        assert_eq!(validate_ca_pem(None).unwrap(), None);
        assert_eq!(validate_ca_pem(Some("")).unwrap(), None);
        assert_eq!(validate_ca_pem(Some("   \n  \t  ")).unwrap(), None);
    }

    #[test]
    fn valid_self_signed_cert_is_accepted() {
        let result = validate_ca_pem(Some(TEST_CA_PEM)).unwrap();
        assert!(result.is_some());
        let pem = result.unwrap();
        assert!(pem.contains("-----BEGIN CERTIFICATE-----"));
        assert!(pem.contains("-----END CERTIFICATE-----"));
    }

    #[test]
    fn valid_chain_two_certs_is_accepted() {
        let chain = format!("{TEST_CA_PEM}\n{TEST_CA_PEM}");
        let result = validate_ca_pem(Some(&chain)).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn private_key_block_is_rejected() {
        let pkcs8 = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEINTuctv5E1hK1bbY8fdp+K06/y/c6w==\n-----END PRIVATE KEY-----\n";
        let err = validate_ca_pem(Some(pkcs8)).unwrap_err();
        match err {
            RanchError::InvalidInput { reason } => {
                assert!(reason.contains("Invalid CA PEM"), "reason: {reason}");
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn mixed_cert_and_key_is_rejected() {
        let pkcs8 = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEINTuctv5E1hK1bbY8fdp+K06/y/c6w==\n-----END PRIVATE KEY-----\n";
        let mixed = format!("{TEST_CA_PEM}\n{pkcs8}");
        let err = validate_ca_pem(Some(&mixed)).unwrap_err();
        match err {
            RanchError::InvalidInput { reason } => {
                assert!(
                    reason.contains("only CERTIFICATE blocks"),
                    "reason: {reason}"
                );
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn garbage_input_is_rejected() {
        let err = validate_ca_pem(Some("this is not a PEM at all")).unwrap_err();
        match err {
            RanchError::InvalidInput { reason } => {
                assert!(reason.contains("Invalid CA PEM"), "reason: {reason}");
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn malformed_pem_markers_only_is_rejected() {
        // Markers but no body — rustls-pemfile silently skips invalid blocks,
        // so this surfaces as "no CERTIFICATE block found".
        let err = validate_ca_pem(Some(
            "-----BEGIN CERTIFICATE-----\nnot-base64\n-----END CERTIFICATE-----",
        ))
        .unwrap_err();
        match err {
            RanchError::InvalidInput { reason } => {
                assert!(reason.contains("Invalid CA PEM"), "reason: {reason}");
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn build_redis_client_no_tls_uses_open_path() {
        let client = build_redis_client("redis://127.0.0.1:6379/0", None);
        assert!(client.is_ok());
    }

    #[test]
    fn build_redis_client_with_custom_ca_constructs_ok() {
        // Constructs the client without opening a connection — verifies
        // rustls-pemfile parses the test cert at build_with_tls time.
        let client = build_redis_client("rediss://example.invalid:6379/0", Some(TEST_CA_PEM));
        assert!(
            client.is_ok(),
            "build_with_tls should accept the test CA: {:?}",
            client.err()
        );
    }

    #[test]
    fn build_redis_client_empty_ca_falls_back_to_open() {
        let client = build_redis_client("rediss://example.invalid:6379/0", Some(""));
        assert!(client.is_ok());
    }
}
