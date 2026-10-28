//! Minimal BullMQ 5 fixtures written straight to Redis: the `meta` hash
//! the schema decoder reads, and job hashes in their state containers.

use redis::aio::MultiplexedConnection;

/// The time every seeded job was queued (and finished, when it did).
pub const SEEDED_AT_MS: i64 = 1_700_000_000_000;

/// Declare `bull:{queue}` as a BullMQ 5 queue.
pub async fn seed_queue(redis: &mut MultiplexedConnection, queue: &str) {
    let _: () = redis::cmd("HSET")
        .arg(format!("bull:{queue}:meta"))
        .arg("version")
        .arg("bullmq:5.0.0")
        .query_async(redis)
        .await
        .unwrap();
}

/// A job named `test-job` in `state` (waiting, active, failed, delayed or
/// completed); declares the queue too. A failed job carries `kaboom` as
/// its reason.
pub async fn seed_job(redis: &mut MultiplexedConnection, queue: &str, id: &str, state: &str) {
    seed_queue(redis, queue).await;
    let hash = format!("bull:{queue}:{id}");
    let mut pipe = redis::pipe();
    pipe.cmd("DEL").arg(&hash);
    let hset = pipe
        .cmd("HSET")
        .arg(&hash)
        .arg("name")
        .arg("test-job")
        .arg("data")
        .arg("{}")
        .arg("opts")
        .arg("{}")
        .arg("timestamp")
        .arg(SEEDED_AT_MS)
        .arg("attemptsMade")
        .arg(0);
    match state {
        "failed" => {
            hset.arg("failedReason")
                .arg("kaboom")
                .arg("stacktrace")
                .arg("[]")
                .arg("finishedOn")
                .arg(SEEDED_AT_MS);
        }
        "completed" => {
            hset.arg("returnvalue")
                .arg("null")
                .arg("finishedOn")
                .arg(SEEDED_AT_MS);
        }
        _ => {}
    }
    match state {
        "waiting" => pipe.cmd("LPUSH").arg(format!("bull:{queue}:wait")).arg(id),
        "active" => pipe
            .cmd("LPUSH")
            .arg(format!("bull:{queue}:active"))
            .arg(id),
        "failed" | "delayed" | "completed" => pipe
            .cmd("ZADD")
            .arg(format!("bull:{queue}:{state}"))
            .arg(SEEDED_AT_MS)
            .arg(id),
        other => panic!("seed_job: unknown state {other}"),
    };
    let _: () = pipe.query_async(redis).await.unwrap();
}

/// Ids in a list container (`wait`, `paused`, `active`).
pub async fn list_ids(redis: &mut MultiplexedConnection, queue: &str, list: &str) -> Vec<String> {
    redis::cmd("LRANGE")
        .arg(format!("bull:{queue}:{list}"))
        .arg(0)
        .arg(-1)
        .query_async(redis)
        .await
        .unwrap()
}

/// Ids in a sorted-set container (`failed`, `delayed`, `completed`…).
pub async fn zset_ids(redis: &mut MultiplexedConnection, queue: &str, zset: &str) -> Vec<String> {
    redis::cmd("ZRANGE")
        .arg(format!("bull:{queue}:{zset}"))
        .arg(0)
        .arg(-1)
        .query_async(redis)
        .await
        .unwrap()
}

pub async fn exists(redis: &mut MultiplexedConnection, key: &str) -> bool {
    redis::cmd("EXISTS")
        .arg(key)
        .query_async::<i64>(redis)
        .await
        .unwrap()
        == 1
}

/// Delete every key of `bull:{queue}`.
pub async fn clear_queue(redis: &mut MultiplexedConnection, queue: &str) {
    super::redis_target::delete_matching(redis, &format!("bull:{queue}:*")).await;
}
