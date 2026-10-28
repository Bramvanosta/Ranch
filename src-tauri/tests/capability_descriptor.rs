//! A kind of backend that declares less than BullMQ: no schedulers, no
//! workers, and a job state of its own (`parked`). What it declares
//! reaches the front end and the MCP tool list as data, operations
//! refuse the rest by name, and the connection test is its own probe.

mod common;

use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use serde_json::json;

use ranch_lib::adapters::fake_backend::FakeBackends;
use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::application::ports::{BackendSpec, BackendSpecs};
use ranch_lib::commands::connections::{
    list_backends_inner, test_connection_inner_with_keychain, TestConnectionArgs,
};
use ranch_lib::commands::queues::{
    list_cron_jobs_inner, list_workers_inner, ListCronJobsArgs, ListWorkersArgs,
};
use ranch_lib::domain::capabilities::{Capabilities, JobActionSupport};
use ranch_lib::domain::endpoint::{BackendKind, Endpoint, EndpointSchema};
use ranch_lib::domain::errors::RanchError;
use ranch_lib::domain::jobs::{BackendState, JobState, Tone};
use ranch_lib::domain::lock::ReadOnlyState;
use ranch_lib::domain::probe::{ProbeError, ProbeReport, ProbeSecrets};
use ranch_lib::domain::ssh_config::SshTunnelConfig;
use ranch_lib::shells::mcp::server::{McpQueueId, QueueInput};
use ranch_lib::shells::mcp::RanchMcpServer;
use rmcp::handler::server::wrapper::Parameters;

const PARKED: JobState = JobState::Other(BackendState {
    code: "parked",
    label: "Parked",
    tone: Tone::Warning,
});

static PARKING: LazyLock<Capabilities> = LazyLock::new(|| Capabilities {
    job_states: vec![
        JobState::Waiting,
        JobState::Active,
        JobState::Completed,
        JobState::Failed,
        PARKED,
    ],
    job_actions: JobActionSupport {
        single: true,
        ..Default::default()
    },
    queue_control: true,
    ..Default::default()
});

/// Jobs wait, run, finish, fail or are parked; one job at a time can be
/// retried or removed, a queue paused. Nothing else.
struct ParkingSpec;

#[async_trait]
impl BackendSpec for ParkingSpec {
    fn kind(&self) -> BackendKind {
        BackendKind::new("parking")
    }
    fn label(&self) -> &str {
        "Parking"
    }
    fn capabilities(&self) -> &Capabilities {
        &PARKING
    }
    fn endpoint_schema(&self) -> EndpointSchema {
        EndpointSchema { fields: Vec::new() }
    }
    fn validate(&self, endpoint: &Endpoint) -> Result<Endpoint, RanchError> {
        Ok(endpoint.clone())
    }
    fn same_server(&self, saved: &Endpoint, new: &Endpoint) -> bool {
        saved == new
    }
    fn tunnel(&self, _: &Endpoint) -> Option<SshTunnelConfig> {
        None
    }
    fn describe(&self, _: &Endpoint) -> String {
        "parking".to_string()
    }
    /// Answers with the password it was given, so a test sees what reached it.
    async fn probe(
        &self,
        _: &Endpoint,
        secrets: &ProbeSecrets,
        _: Arc<dyn SecretStore>,
    ) -> Result<ProbeReport, RanchError> {
        Ok(ProbeReport {
            success: false,
            version: Some("parking 1".into()),
            error: Some(ProbeError::Other {
                reason: format!("password {:?}", secrets.password),
            }),
            host_key: None,
            findings: None,
        })
    }
}

fn parking() -> BackendSpecs {
    BackendSpecs::new(vec![Arc::new(ParkingSpec)])
}

#[test]
fn the_kind_lists_what_it_can_do_and_its_states() {
    let info = list_backends_inner(&parking()).remove(0);
    let state = |code: &str, label: &str, tone: &str, common: bool| json!({ "code": code, "label": label, "tone": tone, "common": common });
    assert_eq!(
        serde_json::to_value(&info.capabilities).unwrap(),
        json!({
            "jobStates": [
                state("waiting", "Waiting", "pending", true),
                state("active", "Active", "running", true),
                state("completed", "Completed", "success", true),
                state("failed", "Failed", "danger", true),
                state("parked", "Parked", "warning", false),
            ],
            "jobActions": { "single": true, "bulk": false, "add": false },
            "maintenance": { "drain": false, "clean": false, "retryAllFailed": false },
            "queueControl": true,
            "schedules": { "list": false, "write": false, "pause": false },
            "workers": { "list": false, "count": false },
            "flows": false,
            "metrics": { "queue": false, "throughput": false, "timeline": false, "native": false },
            "events": { "read": false, "follow": false },
            "server": { "info": false, "topology": false },
        })
    );
}

#[tokio::test]
async fn what_the_kind_does_not_declare_is_refused_by_name() {
    let backends = FakeBackends::of_kind(Arc::new(ParkingSpec));
    backends.add_connection("c", "Parking", ReadOnlyState::Unlocked);
    let queue_id = FakeBackends::queue_id("emails");

    let schedulers = list_cron_jobs_inner(
        ListCronJobsArgs {
            connection_id: "c".into(),
            queue_id: queue_id.clone().into(),
        },
        &backends,
    )
    .await;
    assert!(matches!(
        schedulers,
        Err(RanchError::Unsupported { capability }) if capability == "schedules.list"
    ));

    let workers = list_workers_inner(
        ListWorkersArgs {
            connection_id: "c".into(),
            queue_id: queue_id.into(),
        },
        &backends,
    )
    .await;
    assert!(matches!(
        workers,
        Err(RanchError::Unsupported { capability }) if capability == "workers.list"
    ));
}

#[test]
fn the_mcp_tools_are_those_the_kind_supports() {
    let tools = RanchMcpServer::tools_for(&parking());
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    for absent in [
        "list_schedulers",
        "upsert_scheduler",
        "remove_scheduler",
        "pause_scheduler",
        "resume_scheduler",
        "list_workers",
        "get_flow",
        "get_queue_metrics",
        "get_connection_metrics",
        "bulk_retry_jobs",
        "add_job",
        "drain_queue",
        "clean_queue",
    ] {
        assert!(!names.contains(&absent), "{absent} is listed");
    }
    for present in [
        "list_connections",
        "list_queues",
        "list_jobs",
        "inspect_job",
        "list_failed_jobs",
        "retry_job",
        "remove_job",
        "pause_queue",
        "annotate_cause",
    ] {
        assert!(names.contains(&present), "{present} is missing");
    }

    // The instructions list what is served, and nothing else.
    let instructions = RanchMcpServer::instructions_for(&parking());
    assert!(instructions.contains("READ tools: list_connections, list_environments, list_queues, list_jobs, inspect_job, get_job_logs, list_failed_jobs, list_failure_causes;"));
    assert!(!instructions.contains("list_schedulers"));
    assert!(!instructions.contains("bulk_retry_jobs"));

    let list_jobs = tools.iter().find(|t| t.name == "list_jobs").unwrap();
    let state = &list_jobs.input_schema["properties"]["state"];
    assert_eq!(
        state["enum"],
        json!(["waiting", "active", "completed", "failed", "parked"])
    );
    assert_eq!(
        state["description"],
        "Job state: the common lifecycle (`waiting`, `active`, `completed`, `failed`) or one the connection's backend adds (`parked`)."
    );
}

#[tokio::test]
async fn a_tool_called_anyway_is_refused_by_the_operation() {
    let dir = tempfile::tempdir().unwrap();
    let db = ranch_lib::adapters::sqlite::Db::init(dir.path())
        .await
        .unwrap();
    let backends = FakeBackends::of_kind(Arc::new(ParkingSpec));
    backends.add_connection("c", "Parking", ReadOnlyState::Unlocked);
    let server = RanchMcpServer::new(common::mcp_state(db.pool(), Arc::new(backends)));

    let refused = server
        .list_schedulers(Parameters(QueueInput {
            connection_id: "c".into(),
            queue_id: McpQueueId(FakeBackends::queue_id("emails").as_str().to_string()),
        }))
        .await
        .unwrap_err();
    assert_eq!(refused.data.as_ref().unwrap()["kind"], "unsupported");
    assert!(
        refused.message.contains("schedules.list"),
        "{}",
        refused.message
    );
}

#[tokio::test]
async fn the_connection_test_is_the_probe_of_the_endpoints_kind() {
    let keychain: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let args = |backend: &str| TestConnectionArgs {
        backend: backend.into(),
        endpoint: json!({}),
        password: Some("hunter2".into()),
        ssh_password: None,
        saved_connection_id: None,
    };
    let report = test_connection_inner_with_keychain(args("parking"), &parking(), keychain.clone())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        json!({
            "success": false,
            "redisVersion": "parking 1",
            "error": { "kind": "other", "reason": "password Some(\"hunter2\")" },
            "sshHostKeyPinned": null,
            "probe": null,
        })
    );
    let unknown = test_connection_inner_with_keychain(args("bullmq-redis"), &parking(), keychain)
        .await
        .unwrap_err();
    assert!(
        matches!(&unknown, RanchError::InvalidInput { reason } if reason.contains("bullmq-redis")),
        "{unknown:?}"
    );
}
