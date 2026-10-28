//! An in-memory queue backend.
//!
//! It keeps queues, jobs, schedulers, workers and flows in memory and
//! follows the port's rules as the BullMQ backend does: a write passes the
//! connection's lock (or a human approval) through the write pipeline,
//! acts, and is audited before it replies; what a backend refuses, it
//! refuses with the same errors. Its `memory` kind declares fewer
//! capabilities (no metrics, events or server reads), so it also shows
//! what operations do with a backend that lacks some; a test can make it
//! of another kind ([`FakeBackends::of_kind`]), declaring what that spec
//! declares. It is never registered by a composition root.
//!
//! Tests run the backend contract suite and the operations on it without
//! Redis. Reading it is a short account of what the port promises.

mod backend;
mod model;

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::application::ports::SecretStore;
use crate::application::ports::{
    ApprovalStore, AuditLog, BackendSpec, BackendSpecs, Backends, Capabilities, QueueBackend,
};
use crate::application::{ConnectionStamp, WritePipeline};
use crate::domain::audit::AuditEntry;
use crate::domain::endpoint::{BackendKind, Endpoint, EndpointSchema};
use crate::domain::errors::RanchError;
use crate::domain::lock::ReadOnlyState;
use crate::domain::probe::{ProbeFindings, ProbeReport, ProbeSecrets};
use crate::domain::queues::QueueId;
use crate::domain::ssh_config::SshTunnelConfig;
use backend::FakeBackend;
use model::{FakeQueue, World};

pub use model::FakeJob;

/// Opens fake connections, and lets a test set up and inspect them.
pub struct FakeBackends {
    connections: StdMutex<HashMap<String, Arc<FakeBackend>>>,
    specs: BackendSpecs,
    writes: Arc<WritePipeline>,
    audit: Arc<MemoryAuditLog>,
    approvals: Arc<MemoryApprovals>,
}

impl Default for FakeBackends {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBackends {
    /// Connections of the `memory` kind.
    pub fn new() -> Self {
        Self::of_kind(Arc::new(MemorySpec))
    }

    /// Connections of `spec`'s kind: they declare its capabilities, and
    /// hand out only the families it declares.
    pub fn of_kind(spec: Arc<dyn BackendSpec>) -> Self {
        let audit = Arc::new(MemoryAuditLog::default());
        let approvals = Arc::new(MemoryApprovals::default());
        FakeBackends {
            connections: StdMutex::new(HashMap::new()),
            specs: BackendSpecs::new(vec![spec]),
            writes: Arc::new(WritePipeline::new(audit.clone(), approvals.clone())),
            audit,
            approvals,
        }
    }

    /// A connection with no queues.
    pub fn add_connection(&self, id: &str, name: &str, lock: ReadOnlyState) {
        let backend = FakeBackend {
            stamp: ConnectionStamp {
                id: id.to_string(),
                name: name.to_string(),
                env_color: None,
            },
            capabilities: self.spec().capabilities().clone(),
            writes: Arc::clone(&self.writes),
            world: Mutex::new(World {
                lock,
                queues: Default::default(),
            }),
        };
        self.lock_connections()
            .insert(id.to_string(), Arc::new(backend));
    }

    /// Make the queue named `queue` hold exactly `jobs`, oldest first, and
    /// nothing else. Its id, as the fake lists it.
    pub async fn seed_queue(
        &self,
        connection_id: &str,
        queue: &str,
        jobs: Vec<FakeJob>,
    ) -> QueueId {
        let backend = self.connection(connection_id);
        let mut fresh = FakeQueue::new(queue);
        fresh.jobs = jobs;
        let id = fresh.id.clone();
        backend
            .world
            .lock()
            .await
            .queues
            .insert(queue.to_string(), fresh);
        id
    }

    /// The id the fake gives the queue named `queue`.
    pub fn queue_id(queue: &str) -> QueueId {
        model::queue_id(queue)
    }

    /// A worker named `name` now consumes the queue named `queue`.
    pub async fn add_worker(&self, connection_id: &str, queue: &str, name: &str) {
        let backend = self.connection(connection_id);
        let mut world = backend.world.lock().await;
        if let Some(q) = world.queues.get_mut(queue) {
            q.workers.push(name.to_string());
        }
    }

    /// Every audit row written so far, oldest first.
    pub fn audit(&self) -> Vec<AuditEntry> {
        self.audit.0.lock().expect("audit log").clone()
    }

    /// The approvals the fake's write pipeline trades.
    pub fn approvals(&self) -> &MemoryApprovals {
        &self.approvals
    }

    fn spec(&self) -> &dyn BackendSpec {
        self.specs.iter().next().expect("a fake has one kind")
    }

    fn connection(&self, id: &str) -> Arc<FakeBackend> {
        let connections = self.lock_connections();
        Arc::clone(connections.get(id).expect("add_connection first"))
    }

    fn lock_connections(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<FakeBackend>>> {
        self.connections.lock().expect("fake connections")
    }
}

#[async_trait]
impl Backends for FakeBackends {
    async fn open(&self, connection_id: &str) -> Result<Arc<dyn QueueBackend>, RanchError> {
        let connections = self.lock_connections();
        match connections.get(connection_id) {
            Some(backend) => Ok(Arc::clone(backend) as Arc<dyn QueueBackend>),
            None => Err(RanchError::ConnectionNotFound {
                id: connection_id.to_string(),
            }),
        }
    }

    fn specs(&self) -> &BackendSpecs {
        &self.specs
    }
}

/// The fake's kind, `memory`: its endpoint is any JSON object, since
/// nothing is reached through it, and a probe always finds it there.
pub struct MemorySpec;

#[async_trait]
impl BackendSpec for MemorySpec {
    fn kind(&self) -> BackendKind {
        BackendKind::new("memory")
    }

    fn label(&self) -> &str {
        "In memory"
    }

    fn capabilities(&self) -> &Capabilities {
        &backend::MEMORY_CAPABILITIES
    }

    fn endpoint_schema(&self) -> EndpointSchema {
        EndpointSchema { fields: Vec::new() }
    }

    fn validate(&self, endpoint: &Endpoint) -> Result<Endpoint, RanchError> {
        if endpoint.as_json().is_object() {
            Ok(endpoint.clone())
        } else {
            Err(RanchError::InvalidInput {
                reason: "An endpoint is a JSON object".to_string(),
            })
        }
    }

    fn same_server(&self, saved: &Endpoint, new: &Endpoint) -> bool {
        saved == new
    }

    fn tunnel(&self, _: &Endpoint) -> Option<SshTunnelConfig> {
        None
    }

    fn describe(&self, _: &Endpoint) -> String {
        "in memory".to_string()
    }

    async fn probe(
        &self,
        _: &Endpoint,
        _: &ProbeSecrets,
        _: Arc<dyn SecretStore>,
    ) -> Result<ProbeReport, RanchError> {
        Ok(ProbeReport {
            success: true,
            version: None,
            error: None,
            host_key: None,
            findings: Some(ProbeFindings::default()),
        })
    }
}

/// The audit log, in memory.
#[derive(Default)]
pub struct MemoryAuditLog(StdMutex<Vec<AuditEntry>>);

#[async_trait]
impl AuditLog for MemoryAuditLog {
    async fn append(&self, entry: AuditEntry) -> Result<(), RanchError> {
        self.0.lock().expect("audit log").push(entry);
        Ok(())
    }
}

/// Approvals a test grants: each one is good once, for one connection and
/// one tool.
#[derive(Default)]
pub struct MemoryApprovals(StdMutex<Vec<(String, String, String)>>);

impl MemoryApprovals {
    pub fn approve(&self, approval_id: &str, connection_id: &str, tool: &str) {
        self.0.lock().expect("approvals").push((
            approval_id.to_string(),
            connection_id.to_string(),
            tool.to_string(),
        ));
    }
}

#[async_trait]
impl ApprovalStore for MemoryApprovals {
    async fn consume(
        &self,
        approval_id: &str,
        connection_id: &str,
        tool: &str,
    ) -> Result<bool, RanchError> {
        let mut granted = self.0.lock().expect("approvals");
        let found = granted
            .iter()
            .position(|(a, c, t)| a == approval_id && c == connection_id && t == tool);
        Ok(found.map(|at| granted.remove(at)).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::bullmq_redis::bullmq_schema::{states, JobState};
    use crate::domain::jobs::JobTarget;

    fn reports() -> QueueId {
        model::queue_id("reports")
    }

    /// A parent waiting on two children, one done and one failed.
    async fn flow() -> FakeBackends {
        let backends = FakeBackends::new();
        backends.add_connection("c", "local", ReadOnlyState::Unlocked);
        let at = 1_700_000_000_000;
        let jobs = vec![
            FakeJob::new("parent", states::WAITING_CHILDREN, at),
            FakeJob::new("pdf", JobState::Completed, at).child_of("parent"),
            FakeJob::new("mail", JobState::Failed, at).child_of("parent"),
        ];
        backends.seed_queue("c", "reports", jobs).await;
        backends
    }

    #[tokio::test]
    async fn a_flow_is_read_from_any_of_its_jobs() {
        let backends = flow().await;
        let backend = backends.open("c").await.unwrap();
        let tree = backend
            .flows()
            .unwrap()
            .get_flow(JobTarget {
                queue_id: reports(),
                job_id: "mail".into(),
            })
            .await
            .unwrap();
        assert_eq!(tree.root_key, "memory/reports/parent");
        assert_eq!(tree.focus_key, "memory/reports/mail");
        assert_eq!(tree.nodes.len(), 3);

        let roots = backend
            .flows()
            .unwrap()
            .list_flow_roots(reports())
            .await
            .unwrap();
        assert_eq!(roots.roots.len(), 1);
        let root = &roots.roots[0];
        assert_eq!(root.job_ref.id, "parent");
        assert_eq!(
            (
                root.children_total,
                root.children_completed,
                root.children_failed
            ),
            (2, 1, 1)
        );
    }

    #[tokio::test]
    async fn maintenance_leaves_flow_members_in_place() {
        use crate::domain::audit::Caller;
        use crate::domain::maintenance::QueueMaintenance;

        let backends = flow().await;
        let backend = backends.open("c").await.unwrap();
        let retried = backend
            .maintenance()
            .unwrap()
            .run(
                reports(),
                QueueMaintenance::RetryAllFailed,
                crate::adapters::progress::NoProgress::sink(),
                Caller::User,
            )
            .await
            .unwrap();
        assert_eq!((retried.affected, retried.skipped), (0, 1));
    }
}
