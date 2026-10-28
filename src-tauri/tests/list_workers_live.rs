//! Live tests for the Workers view.
//!
//!   docker compose --profile redis up -d
//!   RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test list_workers_live -- --include-ignored
//!
//! A BullMQ worker is nothing more than a Redis connection named after the
//! queue, so the tests impersonate workers with `CLIENT SETNAME` (exactly
//! what BullMQ's blocking connection does) instead of booting Node.

use base64::Engine;
use ranch_lib::adapters::bullmq_redis::bullmq_schema::QueueRef;
use ranch_lib::adapters::bullmq_redis::connection_actor::{ConnectionConfig, ConnectionRegistry};
use ranch_lib::domain::queues::QueueId;

mod common;
use common::redis_target;

async fn cfg_with(username: Option<&str>, password: Option<&str>) -> ConnectionConfig {
    ConnectionConfig {
        endpoint: ranch_lib::adapters::bullmq_redis::endpoint::RedisEndpoint {
            host: redis_target::host(),
            port: redis_target::port(),
            db: 0,
            username: username.map(Into::into),
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
        },
        password: password.map(Into::into),
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

/// A dedicated (non-multiplexed) connection named like a BullMQ worker.
/// Dropping it disconnects the "worker".
async fn fake_worker(queue: &str, worker_name: Option<&str>) -> redis::aio::MultiplexedConnection {
    let client = redis::Client::open(format!(
        "redis://{}:{}/0",
        redis_target::host(),
        redis_target::port()
    ))
    .expect("redis url");
    let mut conn = client
        .get_multiplexed_async_connection()
        .await
        .expect("redis connection");
    let encoded = base64::engine::general_purpose::STANDARD.encode(queue);
    let name = match worker_name {
        Some(w) => format!("ranch93:{encoded}:w:{w}"),
        None => format!("ranch93:{encoded}"),
    };
    let _: () = redis::cmd("CLIENT")
        .arg("SETNAME")
        .arg(&name)
        .query_async(&mut conn)
        .await
        .expect("setname");
    conn
}

fn args(queue: &str) -> QueueId {
    QueueRef {
        prefix: "ranch93".into(),
        name: queue.into(),
    }
    .id()
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn lists_the_workers_of_this_queue_only_and_sees_them_leave() {
    let registry = ConnectionRegistry::new();
    let cfg = cfg_with(None, None).await;

    let none = registry
        .list_workers("w93", &cfg, args("orders"))
        .await
        .expect("actor send")
        .expect("list_workers");
    assert!(none.supported);
    assert!(none.workers.is_empty(), "no worker attached yet: {none:?}");

    let unnamed = fake_worker("orders", None).await;
    let named = fake_worker("orders", Some("eu-1")).await;
    let _other_queue = fake_worker("invoices", None).await;

    let out = registry
        .list_workers("w93", &cfg, args("orders"))
        .await
        .expect("actor send")
        .expect("list_workers");
    assert!(out.supported);
    assert_eq!(out.unsupported_reason, None);
    assert_eq!(out.workers.len(), 2, "{:?}", out.workers);
    let names: Vec<Option<&str>> = out.workers.iter().map(|w| w.name.as_deref()).collect();
    assert!(
        names.contains(&None) && names.contains(&Some("eu-1")),
        "{names:?}"
    );
    for w in &out.workers {
        assert!(!w.id.is_empty());
        assert!(w.addr.contains(':'), "addr is ip:port, got {}", w.addr);
        assert!(w.age_secs < 60);
        assert!(w.raw.contains_key("name") && w.raw.contains_key("flags"));
    }

    // A worker that disconnects disappears from the next read.
    drop(unnamed);
    drop(named);
    let mut left = usize::MAX;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        left = registry
            .list_workers("w93", &cfg, args("orders"))
            .await
            .expect("actor send")
            .expect("list_workers")
            .workers
            .len();
        if left == 0 {
            break;
        }
    }
    assert_eq!(left, 0, "disconnected workers must not linger");

    registry.shutdown_all().await;
}

/// A provider that forbids `CLIENT LIST` (here an ACL user without
/// `+client`) is reported as a capability, once, not as an error.
#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn a_redis_that_forbids_client_list_is_unsupported_not_an_error() {
    let mut admin = redis::Client::open(format!(
        "redis://{}:{}/0",
        redis_target::host(),
        redis_target::port()
    ))
    .expect("redis url")
    .get_multiplexed_async_connection()
    .await
    .expect("redis connection");
    let _: () = redis::cmd("ACL")
        .arg("SETUSER")
        .arg("ranch93-noclient")
        .arg("on")
        .arg(">ranch93-pass")
        .arg("~*")
        .arg("&*")
        .arg("+@all")
        .arg("-client")
        .query_async(&mut admin)
        .await
        .expect("acl setuser");

    let registry = ConnectionRegistry::new();
    let cfg = cfg_with(Some("ranch93-noclient"), Some("ranch93-pass")).await;
    let _worker = fake_worker("orders-acl", None).await;

    let first = registry
        .list_workers("w93-acl", &cfg, args("orders-acl"))
        .await
        .expect("actor send")
        .expect("a refusal is not an error");
    assert!(!first.supported);
    assert!(first.workers.is_empty());
    let reason = first.unsupported_reason.clone().expect("reason");
    assert!(reason.to_lowercase().contains("noperm"), "{reason}");

    // Probed once: the permission comes back, the cached answer stays (the
    // capability is re-probed when the connection's actor restarts).
    let _: () = redis::cmd("ACL")
        .arg("SETUSER")
        .arg("ranch93-noclient")
        .arg("+client")
        .query_async(&mut admin)
        .await
        .expect("acl grant");
    let second = registry
        .list_workers("w93-acl", &cfg, args("orders-acl"))
        .await
        .expect("actor send")
        .expect("list_workers");
    assert_eq!(
        second, first,
        "the refusal is cached for the actor's lifetime"
    );

    registry.shutdown("w93-acl").await;
    let third = registry
        .list_workers("w93-acl", &cfg, args("orders-acl"))
        .await
        .expect("actor send")
        .expect("list_workers");
    assert!(third.supported, "a fresh actor probes again");
    assert_eq!(third.workers.len(), 1);

    registry.shutdown_all().await;
    let _: () = redis::cmd("ACL")
        .arg("DELUSER")
        .arg("ranch93-noclient")
        .query_async(&mut admin)
        .await
        .expect("acl deluser");
}

/// One `CLIENT LIST` answers for many queues at once.
#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn counts_the_workers_of_many_queues_in_one_read() {
    let registry = ConnectionRegistry::new();
    let cfg = cfg_with(None, None).await;
    let _a = fake_worker("count-a", None).await;
    let _b1 = fake_worker("count-b", Some("eu-1")).await;
    let _b2 = fake_worker("count-b", Some("eu-2")).await;

    let refs: Vec<QueueRef> = ["count-a", "count-b", "count-c"]
        .iter()
        .map(|name| QueueRef {
            prefix: "ranch93".into(),
            name: (*name).into(),
        })
        .collect();
    let out = registry
        .count_queue_workers("w93-count", &cfg, refs.iter().map(|r| r.id()).collect())
        .await
        .expect("actor send")
        .expect("count_queue_workers");
    assert!(out.supported);
    let counts: Vec<(String, u32)> = out
        .counts
        .into_iter()
        .map(|c| (QueueRef::from_id(&c.queue_id).unwrap().name, c.workers))
        .collect();
    assert_eq!(
        counts,
        vec![
            ("count-a".to_string(), 1),
            ("count-b".to_string(), 2),
            ("count-c".to_string(), 0)
        ]
    );
}
