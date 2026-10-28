//! Read handlers, one file per family. Each runs on its own task with a
//! clone of the actor state (see `actor::spawn_read`) and never writes.
//! The helpers here are shared by several families.

pub(super) mod events;
pub(super) mod failed;
pub(super) mod flows;
pub(super) mod jobs;
pub(super) mod metrics;
pub(super) mod queues;
pub(super) mod schedulers;
pub(super) mod server;
pub(super) mod workers;

use std::collections::HashMap;

use tokio_util::sync::CancellationToken;

use super::link::RedisConn;
use ranch_domain::errors::RanchError;

/// One read per item, all sent at once on the multiplexed
/// connection: the round trips overlap, so N queues cost about one. Results
/// come back in the items' order; `None` where a task panicked.
async fn read_each<I, T, F, Fut>(
    conn: &RedisConn,
    items: Vec<I>,
    token: &CancellationToken,
    read: F,
) -> Result<Vec<Option<T>>, RanchError>
where
    F: Fn(RedisConn, I) -> Fut,
    Fut: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let mut out: Vec<Option<T>> = (0..items.len()).map(|_| None).collect();
    let mut set = tokio::task::JoinSet::new();
    for (i, item) in items.into_iter().enumerate() {
        let fut = read(conn.clone(), item);
        set.spawn(async move { (i, fut.await) });
    }
    loop {
        tokio::select! {
            next = set.join_next() => match next {
                None => break,
                Some(Ok((i, value))) => out[i] = Some(value),
                Some(Err(e)) => tracing::error!(error = %e, "read task panicked"),
            },
            _ = token.cancelled() => {
                return Err(RanchError::Cancelled);
            }
        }
    }
    Ok(out)
}

/// A [`read_each`] item whose task panicked, as a read error.
fn read_panicked() -> redis::RedisError {
    redis::RedisError::from((redis::ErrorKind::Client, "read task panicked"))
}

/// Convert `HashMap<String, redis::Value>` (what HGETALL returns when we
/// don't pin the value type) into `HashMap<String, Vec<u8>>`, the shape
/// `JobDecoder::decode_job` expects.
fn redis_value_hash_to_bytes(raw: HashMap<String, redis::Value>) -> HashMap<String, Vec<u8>> {
    raw.into_iter()
        .filter_map(|(k, v)| {
            let bytes = match v {
                redis::Value::BulkString(b) => Some(b),
                redis::Value::SimpleString(s) => Some(s.into_bytes()),
                redis::Value::Int(i) => Some(i.to_string().into_bytes()),
                _ => None,
            };
            bytes.map(|b| (k, b))
        })
        .collect()
}
