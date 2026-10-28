//! The backend contract suite on the in-memory backend: the port's
//! semantics, checked without Redis.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use common::backend_contract::{run_suite, AuditRow, Seeder};
use common::bullmq_seed::SEEDED_AT_MS;
use ranch_lib::adapters::bullmq_redis::bullmq_schema::JobState;
use ranch_lib::adapters::fake_backend::{FakeBackends, FakeJob};
use ranch_lib::domain::lock::ReadOnlyState;
use ranch_lib::domain::queues::QueueId;

struct InMemory {
    backends: FakeBackends,
    next: AtomicUsize,
}

#[async_trait]
impl Seeder for InMemory {
    async fn connection(&self, name: &str, locked: bool) -> String {
        let id = format!("fake-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let lock = if locked {
            ReadOnlyState::Locked
        } else {
            ReadOnlyState::Unlocked
        };
        self.backends.add_connection(&id, name, lock);
        id
    }

    async fn queue(&self, connection_id: &str, queue: &str, jobs: &[(&str, JobState)]) -> QueueId {
        let jobs = jobs
            .iter()
            .map(|(id, state)| FakeJob::new(id, *state, SEEDED_AT_MS))
            .collect();
        self.backends.seed_queue(connection_id, queue, jobs).await
    }

    fn queue_id(&self, queue: &str) -> QueueId {
        FakeBackends::queue_id(queue)
    }

    async fn audit(&self, connection_id: &str) -> Vec<AuditRow> {
        self.backends
            .audit()
            .into_iter()
            .filter(|e| e.connection_id.as_deref() == Some(connection_id))
            .map(|e| AuditRow {
                action: e.action,
                target: e.target,
                outcome: e.outcome,
            })
            .collect()
    }
}

#[tokio::test]
async fn the_in_memory_backend_keeps_the_contract() {
    let seeder = InMemory {
        backends: FakeBackends::new(),
        next: AtomicUsize::new(0),
    };
    run_suite(&seeder.backends, &seeder).await;
}
