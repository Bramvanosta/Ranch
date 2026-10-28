//! `get_topology` against Ranch's own Redis: flow links read
//! from parents' `:dependencies` / `:processed`, the added rate from the
//! event stream, the oldest waiting job. `RANCH_LOCAL_REDIS=1`.

mod common;
use common::repo;

use std::sync::Arc;

use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::queues::{get_topology_inner, GetTopologyArgs};

async fn run(redis: &mut redis::aio::MultiplexedConnection, cmd: &mut redis::Cmd) {
    let _: () = cmd.query_async(redis).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn flows_rates_and_oldest_waiting_are_read() {
    let port: u16 = std::env::var("RANCH_REDIS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6390);
    let dir = tempfile::tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "map".into(),
            backend: "bullmq-redis".into(),
            endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
                host: "127.0.0.1".into(),
                port,
                db: 0,
                username: None,
                tls: false,
                tls_custom_ca: None,
                ssh_tunnel: None,
            }
            .json(),
            password: None,
            ssh_password: None,
            group: GroupSelector::named("Default"),
            environment: EnvironmentSelector::for_color("env-local"),
        },
        &repo(db.pool()),
        &kc,
        &ranch_lib::adapters::bullmq_redis::specs(),
    )
    .await
    .unwrap();
    let run_id = chrono::Utc::now().timestamp_millis();
    let orders = format!("orders{run_id}");
    let emails = format!("emails{run_id}");
    let mut r = redis::Client::open(format!("redis://127.0.0.1:{port}"))
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    for q in [&orders, &emails] {
        run(
            &mut r,
            redis::cmd("HSET")
                .arg(format!("bull:{q}:meta"))
                .arg("version")
                .arg("bullmq:5.81.5"),
        )
        .await;
    }
    // A parent waiting for an email child; a finished one that had two.
    run(
        &mut r,
        redis::cmd("ZADD")
            .arg(format!("bull:{orders}:waiting-children"))
            .arg(now)
            .arg("1"),
    )
    .await;
    run(
        &mut r,
        redis::cmd("SADD")
            .arg(format!("bull:{orders}:1:dependencies"))
            .arg(format!("bull:{emails}:10")),
    )
    .await;
    run(
        &mut r,
        redis::cmd("ZADD")
            .arg(format!("bull:{orders}:completed"))
            .arg(now)
            .arg("2"),
    )
    .await;
    run(
        &mut r,
        redis::cmd("HSET")
            .arg(format!("bull:{orders}:2:processed"))
            .arg(format!("bull:{emails}:11"))
            .arg("\"ok\"")
            .arg(format!("bull:{emails}:12"))
            .arg("\"ok\""),
    )
    .await;
    // Three jobs added in the last hour; one waits since a minute.
    for _ in 0..3 {
        run(
            &mut r,
            redis::cmd("XADD")
                .arg(format!("bull:{emails}:events"))
                .arg("*")
                .arg("event")
                .arg("added")
                .arg("jobId")
                .arg("x"),
        )
        .await;
    }
    run(
        &mut r,
        redis::cmd("LPUSH")
            .arg(format!("bull:{emails}:wait"))
            .arg("20"),
    )
    .await;
    run(
        &mut r,
        redis::cmd("HSET")
            .arg(format!("bull:{emails}:20"))
            .arg("timestamp")
            .arg(now - 60_000),
    )
    .await;

    let topology = get_topology_inner(
        GetTopologyArgs {
            connection_id: conn.id,
        },
        &common::backends(db.pool(), &kc, &ConnectionRegistry::new()),
    )
    .await
    .unwrap();

    for q in [&orders, &emails] {
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(format!("bull:{q}:*"))
            .query_async(&mut r)
            .await
            .unwrap();
        let _: () = redis::cmd("DEL")
            .arg(keys)
            .query_async(&mut r)
            .await
            .unwrap();
    }

    let link = topology
        .links
        .iter()
        .find(|l| l.parent.as_str() == format!("bull:{orders}"))
        .expect("orders hands work to emails");
    assert_eq!(link.child.as_str(), format!("bull:{emails}"));
    assert_eq!(link.jobs, 3);
    let stats = topology
        .queues
        .iter()
        .find(|s| s.queue_id.as_str() == format!("bull:{emails}"))
        .unwrap();
    assert!((stats.added_per_minute - 3.0 / 60.0).abs() < 1e-9);
    assert_eq!(stats.oldest_waiting_ms, Some(now - 60_000));
}
