//! Views read in one or two round trips, whatever the number of
//! queues: a job page (count, range and hashes) and the recent jobs of every
//! queue; the Workers tab's calls share one `CLIENT LIST`. On a slow link
//! against Ranch's own Redis: `RANCH_LOCAL_REDIS=1`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ranch_lib::adapters::bullmq_redis::bullmq_schema::JobState;
use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::jobs::{
    list_jobs_inner, list_recent_jobs_inner, ListJobsArgs, ListRecentJobsArgs,
};
use ranch_lib::commands::queues::{list_queues_inner, list_workers_inner, ListWorkersArgs};

mod common;
use common::{repo, slow_proxy};

const RTT: Duration = Duration::from_millis(600);

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn views_cost_a_round_trip_or_two() {
    let port: u16 = std::env::var("RANCH_REDIS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6390);
    let proxy = slow_proxy(port, RTT / 2).await;
    let dir = tempfile::tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "pipelines".into(),
            backend: "bullmq-redis".into(),
            endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
                host: "127.0.0.1".into(),
                port: proxy,
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
    let registry = ConnectionRegistry::new();
    // Warm: connection open, queues and schema known.
    let queues = list_queues_inner(&conn.id, &common::backends(db.pool(), &kc, &registry))
        .await
        .unwrap();
    let busiest = queues
        .iter()
        .max_by_key(|q| q.counts.get("completed"))
        .expect("the seeded queues");
    assert!(busiest.counts.get("completed") > 0, "{queues:?}");

    let started = Instant::now();
    let page = list_jobs_inner(
        ListJobsArgs {
            connection_id: conn.id.clone(),
            queue_id: busiest.id.clone(),
            state: JobState::Completed.into(),
            offset: 0,
            limit: 50,
            newest_first: true,
            search: None,
            reason: None,
            sort: None,
        },
        &common::backends(db.pool(), &kc, &registry),
    )
    .await
    .unwrap();
    let took = started.elapsed();
    assert!(!page.jobs.is_empty());
    // Before the reads were pipelined: the count, the range, then the hashes: 3 round trips.
    assert!(took < RTT * 5 / 2, "a job page took {took:?}");

    let with_activity = queues
        .iter()
        .filter(|q| q.counts.get("active") + q.counts.get("completed") + q.counts.get("failed") > 0)
        .count();
    assert!(
        with_activity >= 2,
        "the test needs several queues: {queues:?}"
    );
    let started = Instant::now();
    let recent = list_recent_jobs_inner(
        ListRecentJobsArgs {
            connection_id: conn.id.clone(),
            queue_id: None,
            limit: 15,
        },
        &common::backends(db.pool(), &kc, &registry),
    )
    .await
    .unwrap();
    let took = started.elapsed();
    assert!(!recent.jobs.is_empty());
    // The queue list, every queue's ids at once, the hashes: 3 round trips
    // whatever the number of queues (before: one more per queue).
    assert!(
        took < RTT * 7 / 2,
        "recent jobs of {with_activity} queues took {took:?}"
    );
}

/// `CLIENT LIST` calls Redis has served since it started.
async fn client_list_calls(redis: &mut redis::aio::MultiplexedConnection) -> u64 {
    let info: String = redis::cmd("INFO")
        .arg("commandstats")
        .query_async(redis)
        .await
        .unwrap();
    info.lines()
        .find_map(|l| l.strip_prefix("cmdstat_client|list:calls="))
        .and_then(|v| v.split(',').next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn the_workers_of_every_queue_share_one_client_list() {
    let port: u16 = std::env::var("RANCH_REDIS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6390);
    let proxy = slow_proxy(port, RTT / 2).await;
    let dir = tempfile::tempdir().unwrap();
    let db = Db::init(dir.path()).await.unwrap();
    let kc: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let conn = add_connection_inner(
        AddConnectionArgs {
            locked: None,
            name: "workers".into(),
            backend: "bullmq-redis".into(),
            endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
                host: "127.0.0.1".into(),
                port: proxy,
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
    let registry = ConnectionRegistry::new();
    let backends = common::backends(db.pool(), &kc, &registry);
    let queues = list_queues_inner(&conn.id, &backends).await.unwrap();
    let mut probe = redis::Client::open(format!("redis://127.0.0.1:{port}"))
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();

    let before = client_list_calls(&mut probe).await;
    // What the Workers tab does: one call per queue, at once.
    let calls = queues.iter().map(|q| {
        list_workers_inner(
            ListWorkersArgs {
                connection_id: conn.id.clone(),
                queue_id: q.id.clone(),
            },
            &backends,
        )
    });
    for listed in futures_join(calls).await {
        assert!(listed.unwrap().supported);
    }
    let served = client_list_calls(&mut probe).await - before;
    assert!(
        served <= 2,
        "{} queues' workers read CLIENT LIST {served} times",
        queues.len()
    );
}

/// Await every future at once (no `futures` crate in the tree).
async fn futures_join<F: std::future::Future>(futures: impl Iterator<Item = F>) -> Vec<F::Output> {
    let mut pinned: Vec<std::pin::Pin<Box<F>>> = futures.map(Box::pin).collect();
    let mut out: Vec<Option<F::Output>> = pinned.iter().map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (i, f) in pinned.iter_mut().enumerate() {
            if out[i].is_none() {
                match f.as_mut().poll(cx) {
                    std::task::Poll::Ready(v) => out[i] = Some(v),
                    std::task::Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(())
        }
    })
    .await;
    out.into_iter().map(|o| o.unwrap()).collect()
}
