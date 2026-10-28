//! Helpers shared by integration tests. Each test file that needs them
//! declares `mod common;` (a file uses only some of them, hence the
//! `dead_code` allowance).

#![allow(dead_code)]

pub mod audit;
pub mod backend_contract;
pub mod bullmq_seed;
pub mod mcp_stdio;
pub mod redis_target;

use std::sync::Arc;
use std::time::Duration;

use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::bullmq_redis::BullmqRedisBackends;
use ranch_lib::adapters::hmac_sealer::HmacSealer;
use ranch_lib::adapters::progress::NoProgress;
use ranch_lib::adapters::sqlite::{self, SqliteConnectionRepository, SqliteSettingsStore};
use ranch_lib::adapters::system_clock::SystemClock;
use ranch_lib::application::ports::ProgressSink;
use ranch_lib::application::ports::SecretStore;
use sqlx::SqlitePool;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The sealer of a process that seals and checks nothing, which is what a
/// test is unless it says otherwise (`tests/mcp_integrity.rs` does).
pub fn unsealed() -> HmacSealer {
    HmacSealer::unsealed()
}

/// The saved connections over a test's database.
pub fn repo(pool: &SqlitePool) -> SqliteConnectionRepository {
    SqliteConnectionRepository::new(pool.clone(), Arc::new(unsealed()), Arc::new(SystemClock))
}

/// The settings over a test's database.
pub fn kv(pool: &SqlitePool) -> SqliteSettingsStore {
    SqliteSettingsStore::new(pool.clone(), Arc::new(unsealed()), Arc::new(SystemClock))
}

/// Where a test's progress goes: nowhere.
pub fn no_progress() -> Arc<dyn ProgressSink> {
    NoProgress::sink()
}

/// The BullMQ-on-Redis backends over a test's database, keychain and
/// actors: what the shells hand the operations.
pub fn backends(
    pool: &SqlitePool,
    keychain: &Arc<dyn SecretStore>,
    registry: &ConnectionRegistry,
) -> BullmqRedisBackends {
    BullmqRedisBackends::new(
        sqlite::write_pipeline(pool),
        Arc::new(repo(pool)),
        Arc::clone(keychain),
        registry.clone(),
    )
}

/// What `ranch-mcp` serves, over a test's database and backends.
pub fn mcp_state(
    pool: &SqlitePool,
    backends: Arc<dyn ranch_lib::application::ports::Backends>,
) -> ranch_lib::shells::mcp::McpServerState {
    ranch_lib::shells::mcp::McpServerState {
        pool: pool.clone(),
        repository: Arc::new(repo(pool)),
        settings: Arc::new(kv(pool)),
        sealer: Arc::new(unsealed()),
        backends,
    }
}

/// A slow link: a local TCP proxy to Redis that adds `one_way`
/// in each direction (round trip += 2 × `one_way`). Returns its port.
pub async fn slow_proxy(redis_port: u16, one_way: Duration) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let server = TcpStream::connect(("127.0.0.1", redis_port)).await.unwrap();
                let (cr, cw) = client.into_split();
                let (sr, sw) = server.into_split();
                tokio::spawn(delayed_copy(cr, sw, one_way));
                tokio::spawn(delayed_copy(sr, cw, one_way));
            });
        }
    });
    port
}

async fn delayed_copy(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    delay: Duration,
) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(tokio::time::Instant, Vec<u8>)>();
    tokio::spawn(async move {
        while let Some((due, bytes)) = rx.recv().await {
            tokio::time::sleep_until(due).await;
            if to.write_all(&bytes).await.is_err() {
                return;
            }
        }
    });
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match from.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                let _ = tx.send((tokio::time::Instant::now() + delay, buf[..n].to_vec()));
            }
        }
    }
}
