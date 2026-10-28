//! `summarize_window` against Ranch's own Redis: jobs finished
//! in and around a window, read from the completed / failed sets.
//! `RANCH_LOCAL_REDIS=1` + `docker compose --profile redis up -d`.

mod common;
use common::repo;

use std::sync::Arc;

use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use ranch_lib::commands::queues::{summarize_window_inner, SummarizeWindowArgs};

const TIMELINE_DB: u8 = 11;

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn a_window_counts_its_jobs_and_groups_its_failures() {
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
            name: "timeline".into(),
            backend: "bullmq-redis".into(),
            endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
                host: "127.0.0.1".into(),
                port,
                db: TIMELINE_DB,
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

    let queue = format!("timeline-{}", chrono::Utc::now().timestamp_millis());
    let base = format!("bull:{queue}");
    let mut redis = redis::Client::open(format!("redis://127.0.0.1:{port}/{TIMELINE_DB}"))
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let minute = 60_000;
    let _: () = redis::cmd("HSET")
        .arg(format!("{base}:meta"))
        .arg("version")
        .arg("bullmq:5.81.5")
        .query_async(&mut redis)
        .await
        .unwrap();
    // (id, set, finished minutes ago, waited ms, reason)
    let seed = [
        ("1", "completed", 50, 100, ""),
        ("2", "failed", 25, 4_000, "connect ECONNREFUSED 10.0.0.1:25"),
        ("3", "failed", 22, 6_000, "connect ECONNREFUSED 10.0.0.2:25"),
        ("4", "completed", 21, 3_000, ""),
        ("5", "completed", 5, 200, ""),
    ];
    for (id, set, ago, waited, reason) in seed {
        let finished = now - ago * minute;
        let started = finished - 10;
        let _: () = redis::cmd("HSET")
            .arg(format!("{base}:{id}"))
            .arg("name")
            .arg("send")
            .arg("data")
            .arg("{}")
            .arg("timestamp")
            .arg(started - waited)
            .arg("processedOn")
            .arg(started)
            .arg("finishedOn")
            .arg(finished)
            .arg("failedReason")
            .arg(reason)
            .query_async(&mut redis)
            .await
            .unwrap();
        let _: () = redis::cmd("ZADD")
            .arg(format!("{base}:{set}"))
            .arg(finished)
            .arg(id)
            .query_async(&mut redis)
            .await
            .unwrap();
    }

    let summary = summarize_window_inner(
        SummarizeWindowArgs {
            connection_id: conn.id,
            from_ms: now - 30 * minute,
            to_ms: now - 20 * minute,
            range_from_ms: now - 60 * minute,
        },
        &common::backends(db.pool(), &kc, &ConnectionRegistry::new()),
    )
    .await
    .unwrap();

    let keys: Vec<String> = redis::cmd("KEYS")
        .arg(format!("{base}:*"))
        .query_async(&mut redis)
        .await
        .unwrap();
    let _: () = redis::cmd("DEL")
        .arg(keys)
        .query_async(&mut redis)
        .await
        .unwrap();

    assert_eq!((summary.done, summary.failed), (1, 2));
    assert_eq!(summary.wait_p95_ms, Some(6_000));
    assert_eq!(summary.usual_wait_p95_ms, Some(200));
    assert_eq!(summary.causes.len(), 1);
    assert_eq!(summary.causes[0].count, 2);
    assert_eq!(summary.causes[0].jobs[0].id, "3", "newest first");
}
