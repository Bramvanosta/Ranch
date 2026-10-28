//! The actor's way to Redis: the SSH tunnel and the persistent
//! connection ([`Link`]), opened on first use and reopened after a drop,
//! the ping that checks them, and the schema → key layout resolution
//! ([`resolve_keys_for_queue`]) every queue handler starts with.

use std::collections::HashMap;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::actor::{actor_cancelled_result, lock, ActorState, ConnectionConfig};
use crate::bullmq_schema::{
    detect_schema, BullmqSchema, KeyVariant, Keys, QueueRef, V5Keys, V6Keys,
};
use crate::probe::map_redis_err;
use crate::ssh_tunnel::SshTunnel;
use crate::tls::{build_redis_client, connection_manager_config, validate_ca_pem};
use ranch_domain::errors::RanchError;
use ranch_domain::probe::{PingReport, ProbeError};

/// The actor's one Redis connection: multiplexed (concurrent
/// requests share it), reconnects by itself, cheap to clone into a handler.
pub(super) type RedisConn = redis::aio::ConnectionManager;

/// The persistent connection and the tunnel address it was
/// opened through (`None` = direct), so a re-opened tunnel gets a new one.
pub(super) struct CachedRedis {
    conn: RedisConn,
    via: Option<std::net::SocketAddr>,
}

/// The actor's way to Redis: the SSH tunnel and the persistent
/// connection (before it was kept, each command paid a TCP, TLS and
/// SSH-channel handshake). Both opened on first use.
#[derive(Default)]
pub(super) struct Link {
    ssh_tunnel: Option<Arc<SshTunnel>>,
    redis: Option<CachedRedis>,
}

/// Handle a `Ping` message, racing the redis op against the actor's
/// cancellation token.
///
/// Pings over the actor's persistent connection (through the
/// SSH tunnel when there is one), opening it if needed.
pub(super) async fn handle_ping(state: &mut ActorState, token: &CancellationToken) -> PingReport {
    // Validate CA PEM up-front. Bad PEM → typed TLS error.
    if let Err(e) = validate_ca_pem(state.cfg.endpoint.tls_custom_ca.as_deref()) {
        return PingReport {
            success: false,
            latency_ms: None,
            error: Some(ProbeError::TlsHandshake {
                reason: e.to_string(),
            }),
        };
    }

    // The ping goes through the persistent connection (opened
    // now if needed, bounded by its connect timeout); the latency is the
    // PING round trip alone, not a handshake.
    let open = tokio::time::timeout(PING_OPEN_TIMEOUT, open_redis_for_actor(state, token));
    let mut conn = match open.await {
        Ok(Ok(conn)) => conn,
        Ok(Err(RanchError::Redis(e))) => {
            let error = match e.downcast::<redis::RedisError>() {
                Ok(e) => map_redis_err(*e),
                Err(e) => ranch_err_to_test_err(&RanchError::Redis(e)),
            };
            return PingReport {
                success: false,
                latency_ms: None,
                error: Some(error),
            };
        }
        Ok(Err(e)) => {
            return PingReport {
                success: false,
                latency_ms: None,
                error: Some(ranch_err_to_test_err(&e)),
            };
        }
        Err(_) => {
            return PingReport {
                success: false,
                latency_ms: None,
                error: Some(ProbeError::Timeout),
            };
        }
    };

    let ping_cmd = redis::cmd("PING");
    let start = std::time::Instant::now();
    let ping = tokio::time::timeout(
        PING_REPLY_TIMEOUT,
        ping_cmd.query_async::<String>(&mut conn),
    );
    let result = tokio::select! {
        r = ping => r,
        _ = token.cancelled() => return actor_cancelled_result(),
    };
    let latency_ms = start.elapsed().as_millis() as u32;

    match result {
        Ok(Ok(_)) => PingReport {
            success: true,
            latency_ms: Some(latency_ms),
            error: None,
        },
        Ok(Err(e)) => PingReport {
            success: false,
            latency_ms: None,
            error: Some(map_redis_err(e)),
        },
        Err(_) => PingReport {
            success: false,
            latency_ms: None,
            error: Some(ProbeError::Timeout),
        },
    }
}

/// Opening the connection for a ping: the connect timeout plus a
/// cold SSH handshake. Was 2 s direct / 8 s SSH, too short for a slow link.
const PING_OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// One PING on an open connection: a link of up to ~1 s round
/// trip is slow, not unreachable.
const PING_REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Collapse the three SSH RanchError variants into the
/// single `ProbeError::SshTunnel` arm `handle_ping` reports.
fn ranch_err_to_test_err(e: &RanchError) -> ProbeError {
    match e {
        RanchError::SshAuthFailed { reason } => ProbeError::SshTunnel {
            reason: reason.clone(),
        },
        RanchError::SshKeyFileError { path, reason } => ProbeError::SshTunnel {
            reason: format!("Key file at {path}: {reason}"),
        },
        RanchError::SshTunnelFailed { reason } => ProbeError::SshTunnel {
            reason: reason.clone(),
        },
        other => ProbeError::Other {
            reason: other.to_string(),
        },
    }
}

/// get-or-open the cached SSH tunnel. Returns the local
/// loopback address ready for the Redis URL builder.
pub(super) async fn ensure_ssh_tunnel(
    cfg: &ConnectionConfig,
    link: &mut Link,
) -> Result<std::net::SocketAddr, RanchError> {
    if let Some(tunnel) = link.ssh_tunnel.as_ref() {
        return Ok(tunnel.local_addr());
    }
    let ssh_cfg = cfg
        .endpoint
        .ssh_tunnel
        .as_ref()
        .ok_or_else(|| RanchError::SshTunnelFailed {
            reason: "ensure_ssh_tunnel called on a non-SSH connection".to_string(),
        })?;
    let opened = crate::ssh_tunnel::open(
        ssh_cfg,
        cfg.ssh_password.as_deref(),
        (&cfg.endpoint.host, cfg.endpoint.port),
        cfg.keychain.clone(),
        &cfg.connection_id,
    )
    .await?;
    let local = opened.tunnel.local_addr();
    link.ssh_tunnel = Some(Arc::new(opened.tunnel));
    Ok(local)
}

/// The Redis URL through the SSH tunnel's local listener.
pub(super) fn build_redis_url_via_local(
    cfg: &ConnectionConfig,
    local: std::net::SocketAddr,
) -> String {
    cfg.endpoint.url_via(local, cfg.password.as_deref())
}

/// The Redis URL of a direct connection.
pub(super) fn build_redis_url_from_config(cfg: &ConnectionConfig) -> String {
    cfg.endpoint.url(cfg.password.as_deref())
}

/// Map a redis-rs error into a typed `RanchError`. Generic system-error
/// variant; the structured `ProbeError` enum is for ping/test
/// only; queue ops surface the verbatim error for the UI.
pub(super) fn redis_to_ranch_err(e: redis::RedisError) -> RanchError {
    RanchError::redis(e)
}

/// Resolve the `KeyVariant` from cached schema, or detect on the fly via
/// the queue's `:meta` HGETALL (mirrors the schema discovery in
/// [`handle_list_queues`](super::reads::queues::handle_list_queues)).
/// Returns `SchemaNotRecognized` if the queue uses an unsupported BullMQ
/// version.
pub(super) async fn resolve_keys_for_queue(
    state: &mut ActorState,
    conn: &mut RedisConn,
    qref: &QueueRef,
    token: &CancellationToken,
) -> Result<KeyVariant, RanchError> {
    // Reuse the cached schema if a previous list_queues / list_jobs
    // already resolved it.
    let cached = lock(&state.cached_schema).clone();
    if let Some(cached) = cached {
        if let Some(v) = key_variant_for(&cached) {
            return Ok(v);
        }
    }

    // Detect on the fly: HGETALL the queue's meta hash. We use V5Keys to
    // build the meta-key string: `:meta` is identical across V5 and V6
    // so picking either is fine (verified in the Keys tests).
    let meta_key = V5Keys.meta(qref);
    let mut hgetall_cmd = redis::cmd("HGETALL");
    hgetall_cmd.arg(&meta_key);
    let hash: HashMap<String, String> = tokio::select! {
        r = hgetall_cmd.query_async(&mut *conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::Cancelled);
        }
    };
    if hash.is_empty() {
        return Err(RanchError::SchemaNotRecognized {
            detected_version: None,
        });
    }
    let detected = detect_schema(&hash);
    *lock(&state.cached_schema) = Some(detected.clone());
    key_variant_for(&detected).ok_or(RanchError::SchemaNotRecognized {
        detected_version: match detected {
            BullmqSchema::Unknown { detected_version } => detected_version,
            _ => None,
        },
    })
}

fn key_variant_for(schema: &BullmqSchema) -> Option<KeyVariant> {
    match schema {
        BullmqSchema::V5 => Some(KeyVariant::V5(V5Keys)),
        BullmqSchema::V6 => Some(KeyVariant::V6(V6Keys)),
        BullmqSchema::Unknown { .. } => None,
    }
}

/// The actor's Redis connection: the persistent one when it is open, else
/// opened now (validate CA, build the client, connect), racing the actor's
/// cancellation token.
///
/// When the endpoint has an SSH tunnel, the connection goes
/// through the cached SSH tunnel ([`ensure_ssh_tunnel`]), to its loopback
/// listener. A failed connect invalidates the tunnel so the next call
/// re-tunnels.
///
/// Opened once and kept: handlers get a clone. It
/// reconnects by itself after a drop; a closed tunnel drops it with the
/// tunnel, so both are opened again.
pub(super) async fn open_redis_for_actor(
    state: &mut ActorState,
    token: &CancellationToken,
) -> Result<RedisConn, RanchError> {
    let validated_ca = validate_ca_pem(state.cfg.endpoint.tls_custom_ca.as_deref())?;
    // Held while opening, so concurrent reads wait for this
    // one instead of each opening a connection.
    let mut link = state.link.lock().await;
    if link.ssh_tunnel.as_ref().is_some_and(|t| t.is_closed()) {
        link.ssh_tunnel = None;
    }
    let via = if state.cfg.endpoint.ssh_tunnel.is_some() {
        Some(ensure_ssh_tunnel(&state.cfg, &mut link).await?)
    } else {
        None
    };
    if let Some(cached) = link.redis.as_ref() {
        if cached.via == via {
            return Ok(cached.conn.clone());
        }
    }
    link.redis = None;

    // SSH-tunneled connections use redis:// (no TLS) over loopback:
    // the SSH session is the encryption layer. validated_ca only
    // applies to the direct path.
    let (url, ca_for_call) = match via {
        Some(local) => (build_redis_url_via_local(&state.cfg, local), None),
        None => (
            build_redis_url_from_config(&state.cfg),
            validated_ca.as_deref(),
        ),
    };
    let client = build_redis_client(&url, ca_for_call).map_err(redis_to_ranch_err)?;

    let result = tokio::select! {
        r = RedisConn::new_with_config(client, connection_manager_config()) => r.map_err(redis_to_ranch_err),
        _ = token.cancelled() => Err(RanchError::Cancelled),
    };

    match result {
        Ok(conn) => {
            link.redis = Some(CachedRedis {
                conn: conn.clone(),
                via,
            });
            Ok(conn)
        }
        Err(e) => {
            // The failure may be on the SSH side: re-tunnel next time.
            if via.is_some() {
                link.ssh_tunnel = None;
            }
            Err(e)
        }
    }
}
