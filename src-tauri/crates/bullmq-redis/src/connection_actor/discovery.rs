//! Queue discovery: SCAN for BullMQ `*:meta` hashes, detect each queue's
//! schema, and keep the result in the actor's registry, served from the
//! cache and refreshed in the background once it is older than
//! [`REGISTRY_TTL`].

use std::collections::HashMap;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::actor::ActorState;
use super::link::{redis_to_ranch_err, RedisConn};
use crate::bullmq_schema::{detect_schema, BullmqSchema, QueueRef};
use ranch_domain::errors::RanchError;

/// How old the queue registry may get before a refresh starts.
/// It is served while it refreshes, so a new queue shows up within ~30 s.
pub(super) const REGISTRY_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// The BullMQ queues of a Redis (meta key with a version) and
/// their schema, as the last SCAN found them.
#[derive(Clone)]
pub(super) struct QueueRegistry {
    pub(super) queues: Vec<(QueueRef, BullmqSchema)>,
    pub(super) scanned_at: std::time::Instant,
}

#[derive(Default)]
pub(super) struct RegistrySlot {
    pub(super) current: Option<QueueRegistry>,
    pub(super) refreshing: bool,
}

/// The queues of this Redis. The first call scans; later calls
/// answer from the cache at once, and once it is older than
/// [`REGISTRY_TTL`] a background task rescans (one at a time) while the
/// cached list keeps being served.
pub(super) async fn queue_registry(
    state: &mut ActorState,
    conn: &RedisConn,
    token: &CancellationToken,
) -> Result<Vec<(QueueRef, BullmqSchema)>, RanchError> {
    let slot = Arc::clone(&state.queue_registry);
    let (cached, refresh) = {
        let mut guard = slot.lock().map_err(|_| RanchError::InvalidInput {
            reason: "queue registry poisoned".to_string(),
        })?;
        let cached = guard.current.clone();
        let stale = cached
            .as_ref()
            .is_some_and(|r| r.scanned_at.elapsed() >= REGISTRY_TTL);
        let refresh = stale && !guard.refreshing;
        if refresh {
            guard.refreshing = true;
        }
        (cached, refresh)
    };

    let Some(cached) = cached else {
        let mut conn = conn.clone();
        let queues = discover_queues(&mut conn, token).await?;
        if let Ok(mut guard) = slot.lock() {
            guard.current = Some(QueueRegistry {
                queues: queues.clone(),
                scanned_at: std::time::Instant::now(),
            });
        }
        return Ok(queues);
    };

    if refresh {
        let mut conn = conn.clone();
        let token = token.clone();
        tokio::spawn(async move {
            let found = discover_queues(&mut conn, &token).await;
            if let Ok(mut guard) = slot.lock() {
                guard.refreshing = false;
                match found {
                    Ok(queues) => {
                        guard.current = Some(QueueRegistry {
                            queues,
                            scanned_at: std::time::Instant::now(),
                        });
                    }
                    Err(e) => tracing::warn!(error = %e, "queue registry refresh failed"),
                }
            }
        });
    }
    Ok(cached.queues)
}

/// SCAN for `*:meta`, then every meta hash in one pipeline:
/// the BullMQ queues (a `version` field) and their schema.
async fn discover_queues(
    conn: &mut RedisConn,
    token: &CancellationToken,
) -> Result<Vec<(QueueRef, BullmqSchema)>, RanchError> {
    let candidates: Vec<(String, QueueRef)> = scan_meta_keys(conn, token)
        .await?
        .into_iter()
        .filter_map(|key| {
            let (prefix, name) = parse_queue_meta_key(&key)?;
            Some((key, QueueRef { prefix, name }))
        })
        .collect();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let mut pipe = redis::pipe();
    for (key, _) in &candidates {
        pipe.cmd("HGETALL").arg(key);
    }
    let hashes: Vec<HashMap<String, String>> = tokio::select! {
        r = pipe.query_async(conn) => r.map_err(redis_to_ranch_err)?,
        _ = token.cancelled() => {
            return Err(RanchError::Cancelled);
        }
    };
    Ok(candidates
        .into_iter()
        .zip(hashes)
        // Skip non-BullMQ `*:meta` hashes (no `version` field, or empty).
        .filter(|(_, hash)| hash.contains_key("version"))
        .map(|((_, qref), hash)| (qref, detect_schema(&hash)))
        .collect())
}

/// Iterate `SCAN MATCH *:meta` until cursor returns 0, collecting all
/// matching keys. Bounded by the actor's cancellation token.
async fn scan_meta_keys(
    conn: &mut RedisConn,
    token: &CancellationToken,
) -> Result<Vec<String>, RanchError> {
    let mut cursor: u64 = 0;
    let mut out: Vec<String> = Vec::new();
    loop {
        let mut scan_cmd = redis::cmd("SCAN");
        scan_cmd
            .arg(cursor)
            .arg("MATCH")
            .arg("*:meta")
            .arg("COUNT")
            // One round trip per COUNT keys: 100 made a large
            // keyspace cost thousands of round trips.
            .arg(SCAN_COUNT);
        let (next, keys): (u64, Vec<String>) = tokio::select! {
            r = scan_cmd.query_async(&mut *conn) => r.map_err(redis_to_ranch_err)?,
            _ = token.cancelled() => {
                return Err(RanchError::Cancelled);
            }
        };
        out.extend(keys);
        cursor = next;
        if cursor == 0 {
            break;
        }
    }
    Ok(out)
}

/// Parse a `bull:emails:meta` key into `(prefix, name)`. Last segment MUST
/// be `"meta"`. We split on the LAST `:` before the `:meta` suffix to
/// extract the queue name; everything before is the prefix.
///
/// Honors custom prefixes:
///   `bull:emails:meta`              → ("bull", "emails")
///   `myapp:queues:billing:meta`     → ("myapp:queues", "billing")
///   `bull:meta`                     → None (no name segment)
///   `:meta`                         → None (empty everywhere)
///   `bull:emails`                   → None (no `:meta` suffix)
fn parse_queue_meta_key(key: &str) -> Option<(String, String)> {
    let trimmed = key.strip_suffix(":meta")?;
    let last_colon = trimmed.rfind(':')?;
    let prefix = &trimmed[..last_colon];
    let name = &trimmed[last_colon + 1..];
    if prefix.is_empty() || name.is_empty() {
        return None;
    }
    Some((prefix.to_string(), name.to_string()))
}

/// First-success-wins schema cache. If `state.cached_schema` already holds
/// V5 or V6, returns it. Otherwise picks the first non-Unknown variant
/// from `detected`, caches it, and returns. If every detected queue has
/// an Unknown schema, returns a typed `SchemaNotRecognized` error
/// carrying the first detected version string for the UI to surface.
pub(super) fn pick_or_cache_schema(
    cached: &mut Option<BullmqSchema>,
    detected: &[(QueueRef, BullmqSchema)],
) -> Result<BullmqSchema, RanchError> {
    if let Some(s) = cached.clone() {
        if !matches!(s, BullmqSchema::Unknown { .. }) {
            return Ok(s);
        }
    }
    for (_, schema) in detected {
        if !matches!(schema, BullmqSchema::Unknown { .. }) {
            *cached = Some(schema.clone());
            return Ok(schema.clone());
        }
    }
    // Every detected queue had Unknown: surface the first detected_version.
    let first_unknown = detected
        .iter()
        .find_map(|(_, s)| match s {
            BullmqSchema::Unknown { detected_version } => Some(detected_version.clone()),
            _ => None,
        })
        .unwrap_or(None);
    Err(RanchError::SchemaNotRecognized {
        detected_version: first_unknown,
    })
}

/// Keys looked at per SCAN round trip.
const SCAN_COUNT: u32 = 1_000;

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_queue_meta_key ────────────────────────────────────────────

    #[test]
    fn parse_default_prefix() {
        assert_eq!(
            parse_queue_meta_key("bull:emails:meta"),
            Some(("bull".to_string(), "emails".to_string()))
        );
    }

    #[test]
    fn parse_custom_prefix() {
        assert_eq!(
            parse_queue_meta_key("myapp:queues:billing:meta"),
            Some(("myapp:queues".to_string(), "billing".to_string()))
        );
    }

    #[test]
    fn parse_rejects_no_meta_suffix() {
        assert_eq!(parse_queue_meta_key("bull:emails"), None);
    }

    #[test]
    fn parse_rejects_only_meta() {
        assert_eq!(parse_queue_meta_key(":meta"), None);
        assert_eq!(parse_queue_meta_key("meta"), None);
    }

    #[test]
    fn parse_rejects_no_name_segment() {
        // "bull:meta" → strip ":meta" → "bull" → no `:` → None.
        assert_eq!(parse_queue_meta_key("bull:meta"), None);
    }

    #[test]
    fn parse_rejects_empty_prefix() {
        // ":emails:meta" → strip ":meta" → ":emails" → last `:` at idx 0 →
        // prefix "" empty → None.
        assert_eq!(parse_queue_meta_key(":emails:meta"), None);
    }

    // ── pick_or_cache_schema ────────────────────────────────────────────

    fn qref(prefix: &str, name: &str) -> QueueRef {
        QueueRef {
            prefix: prefix.into(),
            name: name.into(),
        }
    }

    #[test]
    fn pick_uses_cached_when_present() {
        let mut cached = Some(BullmqSchema::V5);
        let detected = vec![(qref("bull", "x"), BullmqSchema::V6)]; // ignored
        let s = pick_or_cache_schema(&mut cached, &detected).unwrap();
        assert_eq!(s, BullmqSchema::V5);
    }

    #[test]
    fn pick_caches_first_known_schema() {
        let mut cached: Option<BullmqSchema> = None;
        let detected = vec![
            (
                qref("bull", "a"),
                BullmqSchema::Unknown {
                    detected_version: Some("4.x".into()),
                },
            ),
            (qref("bull", "b"), BullmqSchema::V6),
            (qref("bull", "c"), BullmqSchema::V5),
        ];
        let s = pick_or_cache_schema(&mut cached, &detected).unwrap();
        assert_eq!(s, BullmqSchema::V6);
        assert_eq!(cached, Some(BullmqSchema::V6));
    }

    #[test]
    fn pick_returns_schema_not_recognized_when_all_unknown() {
        let mut cached: Option<BullmqSchema> = None;
        let detected = vec![
            (
                qref("bull", "a"),
                BullmqSchema::Unknown {
                    detected_version: Some("7.0.0".into()),
                },
            ),
            (
                qref("bull", "b"),
                BullmqSchema::Unknown {
                    detected_version: Some("8.x".into()),
                },
            ),
        ];
        let err = pick_or_cache_schema(&mut cached, &detected).unwrap_err();
        match err {
            RanchError::SchemaNotRecognized { detected_version } => {
                assert_eq!(detected_version.as_deref(), Some("7.0.0"));
            }
            other => panic!("expected SchemaNotRecognized, got {other:?}"),
        }
    }

    #[test]
    fn pick_re_attempts_when_cached_is_unknown() {
        let mut cached = Some(BullmqSchema::Unknown {
            detected_version: Some("?".into()),
        });
        let detected = vec![(qref("bull", "a"), BullmqSchema::V5)];
        let s = pick_or_cache_schema(&mut cached, &detected).unwrap();
        assert_eq!(s, BullmqSchema::V5);
        assert_eq!(cached, Some(BullmqSchema::V5));
    }
}
