//! The backend contract suite on BullMQ over Redis, through the same
//! connection actors the app uses. Each section seeds its own
//! `bull:ranch-contract-*` queue and saved connection.
//!
//! ```bash
//! docker compose --profile redis up -d
//! RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 \
//!   cargo test --test backend_contract_live -- --include-ignored
//! ```

mod common;
use common::repo;

use std::sync::Arc;

use async_trait::async_trait;
use common::backend_contract::{run_suite, AuditRow, Seeder};
use common::{audit, bullmq_seed, redis_target};
use ranch_lib::adapters::bullmq_redis::bullmq_schema::{JobState, QueueId, QueueRef};
use ranch_lib::adapters::bullmq_redis::connection_actor::ConnectionRegistry;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::adapters::sqlite::Db;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{add_connection_inner, AddConnectionArgs};
use ranch_lib::commands::hierarchy::{EnvironmentSelector, GroupSelector};
use tokio::sync::Mutex;

struct OnRedis {
    db: Db,
    keychain: Arc<dyn SecretStore>,
    redis: Mutex<redis::aio::MultiplexedConnection>,
}

#[async_trait]
impl Seeder for OnRedis {
    async fn connection(&self, name: &str, locked: bool) -> String {
        let args = AddConnectionArgs {
            name: name.into(),
            backend: "bullmq-redis".into(),
            endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
                host: redis_target::host(),
                port: redis_target::port(),
                db: 0,
                username: None,
                tls: false,
                tls_custom_ca: None,
                ssh_tunnel: None,
            }
            .json(),
            password: None,
            ssh_password: None,
            group: GroupSelector::named("Contract"),
            environment: EnvironmentSelector::for_color("env-local"),
            locked: Some(locked),
        };
        add_connection_inner(
            args,
            &repo(self.db.pool()),
            &self.keychain,
            &ranch_lib::adapters::bullmq_redis::specs(),
        )
        .await
        .expect("add connection")
        .id
    }

    async fn queue(&self, _connection_id: &str, queue: &str, jobs: &[(&str, JobState)]) -> QueueId {
        let mut redis = self.redis.lock().await;
        bullmq_seed::clear_queue(&mut redis, queue).await;
        bullmq_seed::seed_queue(&mut redis, queue).await;
        for (id, state) in jobs {
            bullmq_seed::seed_job(&mut redis, queue, id, state.as_str()).await;
        }
        self.queue_id(queue)
    }

    fn queue_id(&self, queue: &str) -> QueueId {
        QueueRef {
            prefix: "bull".into(),
            name: queue.into(),
        }
        .id()
    }

    async fn audit(&self, connection_id: &str) -> Vec<AuditRow> {
        let mut rows: Vec<AuditRow> = audit::rows(self.db.pool(), connection_id)
            .await
            .into_iter()
            .map(|r| AuditRow {
                action: r.action,
                target: r.target,
                outcome: r.outcome,
            })
            .collect();
        rows.reverse();
        rows
    }
}

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn bullmq_on_redis_keeps_the_contract() {
    let dir = tempfile::tempdir().unwrap();
    let seeder = OnRedis {
        db: Db::init(dir.path()).await.unwrap(),
        keychain: Arc::new(InMemoryKeychain::new()),
        redis: Mutex::new(redis_target::connect().await),
    };
    let registry = ConnectionRegistry::new();
    let backends = common::backends(seeder.db.pool(), &seeder.keychain, &registry);
    run_suite(&backends, &seeder).await;
    registry.shutdown_all().await;
}
