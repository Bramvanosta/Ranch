//! Probing a Redis before its connection is saved: a one-shot connection
//! (through the SSH tunnel when there is one), `PING` and `INFO server`, a
//! classified error when it fails, and a bounded look at what BullMQ holds
//! there.

use std::sync::Arc;

use super::endpoint::RedisEndpoint;
use crate::tls::build_redis_client;
use ranch_application::ports::SecretStore;
use ranch_domain::errors::RanchError;
use ranch_domain::probe::{HostKeyPin, ProbeError, ProbeFindings, ProbeReport, ProbeSecrets};

/// Bounded discovery for the probe: a few `SCAN` rounds, a sample of names.
const PROBE_SCAN_ROUNDS: usize = 10;
const PROBE_SAMPLE: usize = 5;

/// Collapse the three SSH RanchError variants into the
/// single `ProbeError::SshTunnel` arm used by the form. The
/// form's `renderTestError` switch displays the reason verbatim, no
/// need to fan out three arms there.
fn ssh_error(e: &RanchError) -> Option<ProbeError> {
    match e {
        RanchError::SshAuthFailed { reason } => Some(ProbeError::SshTunnel {
            reason: reason.clone(),
        }),
        RanchError::SshKeyFileError { path, reason } => Some(ProbeError::SshTunnel {
            reason: format!("Key file at {path}: {reason}"),
        }),
        RanchError::SshTunnelFailed { reason } => Some(ProbeError::SshTunnel {
            reason: reason.clone(),
        }),
        _ => None,
    }
}

/// Probe the Redis `ep` points at. Opens nothing that outlives the call.
pub(super) async fn probe(
    ep: &RedisEndpoint,
    secrets: &ProbeSecrets,
    keychain: Arc<dyn SecretStore>,
) -> Result<ProbeReport, RanchError> {
    let password = secrets.password.as_deref();

    // When an SSH tunnel is configured, open it once for
    // this probe, route the Redis PING through it, then drop it. We do
    // NOT cache anything here: a probe is exploratory + one-shot. The
    // bulk of SSH-related latency lives in this branch (cold open).
    if let Some(ssh_cfg) = ep.ssh_tunnel.as_ref() {
        // Use a synthetic connection id for the keychain hostkey pin.
        // The probe doesn't know the eventual connection_id (the
        // user is still filling the form), so we key on a stable
        // "test::{host}:{port}" name. Real connects use the real id
        // once the connection is saved.
        let synthetic_id = format!("test::{}:{}", ssh_cfg.host, ssh_cfg.port);
        let open_result = crate::ssh_tunnel::open(
            ssh_cfg,
            secrets.ssh_password.as_deref(),
            (&ep.host, ep.port),
            keychain,
            &synthetic_id,
        )
        .await;
        match open_result {
            Ok(opened) => {
                let local = opened.tunnel.local_addr();
                let url = ep.url_via(local, password);
                let result = tokio::time::timeout(
                    std::time::Duration::from_millis(20_000),
                    ping_and_version(&url, None),
                )
                .await;
                let pin = Some(HostKeyPin {
                    fingerprint: opened.fingerprint,
                    was_new_pin: opened.was_new_pin,
                });
                // tunnel drops at end-of-scope; one-shot probe.
                drop(opened.tunnel);
                return Ok(match result {
                    Ok(Ok((version, findings))) => ProbeReport {
                        success: true,
                        version,
                        error: None,
                        host_key: pin,
                        findings: Some(findings),
                    },
                    Ok(Err(e)) => ProbeReport {
                        success: false,
                        version: None,
                        error: Some(map_redis_err(e)),
                        host_key: pin,
                        findings: None,
                    },
                    Err(_elapsed) => ProbeReport {
                        success: false,
                        version: None,
                        error: Some(ProbeError::Timeout),
                        host_key: pin,
                        findings: None,
                    },
                });
            }
            Err(err) => {
                if let Some(error) = ssh_error(&err) {
                    return Ok(ProbeReport {
                        success: false,
                        version: None,
                        error: Some(error),
                        host_key: None,
                        findings: None,
                    });
                }
                return Err(err);
            }
        }
    }

    let url = ep.url(password);

    let result = tokio::time::timeout(
        // The probe also looks at what BullMQ holds: a little longer.
        // And a link of up to ~1 s round trip is slow, not down.
        std::time::Duration::from_millis(15_000),
        ping_and_version(&url, ep.tls_custom_ca.as_deref()),
    )
    .await;

    Ok(match result {
        Ok(Ok((version, findings))) => ProbeReport {
            success: true,
            version,
            error: None,
            host_key: None,
            findings: Some(findings),
        },
        Ok(Err(e)) => ProbeReport {
            success: false,
            version: None,
            error: Some(map_redis_err(e)),
            host_key: None,
            findings: None,
        },
        Err(_elapsed) => ProbeReport {
            success: false,
            version: None,
            error: Some(ProbeError::Timeout),
            host_key: None,
            findings: None,
        },
    })
}

async fn ping_and_version(
    url: &str,
    tls_custom_ca: Option<&str>,
) -> Result<(Option<String>, ProbeFindings), redis::RedisError> {
    let client = build_redis_client(url, tls_custom_ca)?;
    let mut conn = client
        .get_multiplexed_async_connection_with_config(&crate::tls::async_connection_config())
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
    // leaves its part empty, never fails the probe.
    let mut findings = ProbeFindings {
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
    findings.scan_capped = cursor != 0;
    queues.sort();
    queues.dedup();
    findings.queues_found = queues.len() as u32;
    findings.queue_sample = queues.iter().take(PROBE_SAMPLE).cloned().collect();
    if !queues.is_empty() {
        let mut pipe = redis::pipe();
        for q in queues.iter().take(200) {
            pipe.cmd("ZCARD").arg(format!("{q}:repeat"));
        }
        let counts: Result<Vec<u64>, _> = pipe.query_async(&mut conn).await;
        findings.schedulers = counts.map(|c| c.iter().sum::<u64>() as u32).unwrap_or(0);
    }
    let list: Result<String, _> = redis::cmd("CLIENT")
        .arg("LIST")
        .query_async(&mut conn)
        .await;
    findings.workers = list.ok().map(|l| {
        // A queue's key base is its BullMQ queue id.
        let refs: Vec<crate::bullmq_schema::QueueRef> = queues
            .iter()
            .filter_map(|q| crate::bullmq_schema::QueueRef::parse(q).ok())
            .collect();
        crate::bullmq_schema::workers::count_workers(&l, &refs)
            .iter()
            .map(|c| c.workers)
            .sum()
    });
    Ok((version, findings))
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

/// A redis-rs error as a probe or a ping reports it.
pub(crate) fn map_redis_err(e: redis::RedisError) -> ProbeError {
    use redis::ErrorKind;
    let kind = e.kind();
    let msg = e.to_string();
    let lower = msg.to_lowercase();

    if TLS_NEEDLES.iter().any(|needle| lower.contains(needle)) {
        return ProbeError::TlsHandshake { reason: msg };
    }

    match kind {
        ErrorKind::AuthenticationFailed => ProbeError::AuthFailed,
        // redis-rs 1.x renamed `IoError` → `Io`. This catches platform-level
        // network failures (refused, no route to host, dns failure, etc.).
        ErrorKind::Io => ProbeError::CannotReach { reason: msg },
        _ => ProbeError::Other { reason: msg },
    }
}
