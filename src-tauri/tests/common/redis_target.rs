//! The Redis the live suites run against: Ranch's own (`docker compose
//! --profile redis up -d`, port 6390) or CI's service, chosen by
//! `RANCH_REDIS_HOST` and `RANCH_REDIS_PORT`.

pub fn host() -> String {
    std::env::var("RANCH_REDIS_HOST").unwrap_or_else(|_| "127.0.0.1".into())
}

pub fn port() -> u16 {
    std::env::var("RANCH_REDIS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6390)
}

pub fn url() -> String {
    format!("redis://{}:{}", host(), port())
}

/// A connection for seeding and inspecting keys around the code under test.
pub async fn connect() -> redis::aio::MultiplexedConnection {
    redis::Client::open(url())
        .expect("a valid Redis URL")
        .get_multiplexed_async_connection()
        .await
        .expect("Redis reachable (RANCH_REDIS_HOST / RANCH_REDIS_PORT)")
}

/// Every key under `prefix:queue:`, sorted.
pub async fn keys_of(redis: &mut redis::aio::MultiplexedConnection, pattern: &str) -> Vec<String> {
    let mut keys: Vec<String> = redis::cmd("KEYS")
        .arg(pattern)
        .query_async(redis)
        .await
        .unwrap();
    keys.sort();
    keys
}

/// Delete every key matching `pattern`.
pub async fn delete_matching(redis: &mut redis::aio::MultiplexedConnection, pattern: &str) {
    let keys = keys_of(redis, pattern).await;
    if !keys.is_empty() {
        let _: () = redis::cmd("DEL")
            .arg(keys)
            .query_async(redis)
            .await
            .unwrap();
    }
}

/// An actor configuration on this Redis: unlocked, an in-memory keychain and
/// database. For the suites that drive a connection actor directly.
pub async fn connection_config(
) -> ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionConfig {
    ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionConfig {
        endpoint: ranch_lib::adapters::bullmq_redis::endpoint::RedisEndpoint::new(host(), port()),
        password: None,
        ssh_password: None,
        keychain: std::sync::Arc::new(ranch_lib::adapters::secrets::InMemoryKeychain::new()),
        initial_lock_state: ranch_lib::domain::lock::ReadOnlyState::Unlocked,
        connection_id: "test-conn".into(),
        connection_name: "test".into(),
        env_color: Some("env-local".into()),
        writes: ranch_lib::adapters::sqlite::write_pipeline(
            &sqlx::SqlitePool::connect("sqlite::memory:")
                .await
                .expect("in-memory pool"),
        ),
    }
}
