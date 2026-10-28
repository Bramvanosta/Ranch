//! **real BullMQ workers process what Ranch adds**.
//!
//! Ranch writes jobs with its own Lua script, not with BullMQ's. This test is
//! the proof that the two agree: for BullMQ 5.x *and* 6.x it starts a real
//! `Worker` (Node, `.docker/bullmq-compat/worker.mjs`), adds a standard, a
//! LIFO, two prioritized and a delayed job through the connection actor, and
//! checks what BullMQ itself makes of them: counts, parsed jobs, processing
//! order, payload round-trip, events.
//!
//! ```bash
//! docker compose --profile redis up -d
//! (cd .docker/bullmq-compat && npm ci)
//! RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test add_job_compat_live -- --include-ignored
//! ```
//!
//! Skipped without `RANCH_LOCAL_REDIS`. With `RANCH_BULLMQ_COMPAT=1` (CI) a
//! missing Node harness is a failure, not a skip.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use ranch_lib::adapters::bullmq_redis::bullmq_schema::{states, JobState, QueueRef};
use ranch_lib::adapters::bullmq_redis::connection_actor::{ConnectionConfig, ConnectionRegistry};
use ranch_lib::domain::audit::Caller;
use ranch_lib::domain::jobs::NewJob;

mod common;
use common::redis_target;

const PREFIX: &str = "ranchcompat";

/// The audit trail needs a migrated database; the temp dir must outlive it.
async fn local_redis_cfg() -> (tempfile::TempDir, ConnectionConfig) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = ranch_lib::adapters::sqlite::Db::init(dir.path())
        .await
        .expect("db");
    let cfg = ConnectionConfig {
        endpoint: ranch_lib::adapters::bullmq_redis::endpoint::RedisEndpoint {
            host: redis_target::host(),
            port: redis_target::port(),
            db: 0,
            username: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
        },
        password: None,
        ssh_password: None,
        keychain: std::sync::Arc::new(ranch_lib::adapters::secrets::InMemoryKeychain::new()),
        initial_lock_state: ranch_lib::domain::lock::ReadOnlyState::Unlocked,
        connection_id: "compat-conn".into(),
        connection_name: "compat".into(),
        env_color: Some("env-local".into()),
        writes: ranch_lib::adapters::sqlite::write_pipeline(&db.pool().clone()),
    };
    (dir, cfg)
}

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.docker/bullmq-compat")
}

/// `None` = skip. Panics instead when the run is declared CI-blocking.
fn harness_or_skip() -> Option<PathBuf> {
    let dir = harness_dir();
    let blocking = std::env::var("RANCH_BULLMQ_COMPAT").is_ok();
    let installed =
        dir.join("node_modules/bullmq5").exists() && dir.join("node_modules/bullmq6").exists();
    let node = Command::new("node").arg("--version").output().is_ok();
    if installed && node {
        return Some(dir);
    }
    let why = "BullMQ compat harness missing: run `npm ci` in .docker/bullmq-compat (needs Node)";
    assert!(!blocking, "{why}");
    eprintln!("skipping: {why}");
    None
}

/// A running `worker.mjs`, read line by line.
struct Harness {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    log: Vec<String>,
}

impl Harness {
    fn start(dir: &PathBuf, major: u8, queue: &str, expected: usize) -> Self {
        let mut child = Command::new("node")
            .arg("worker.mjs")
            .arg(major.to_string())
            .arg(queue)
            .arg(expected.to_string())
            .arg("20000")
            .current_dir(dir)
            .env("RANCH_REDIS_HOST", redis_target::host())
            .env("RANCH_REDIS_PORT", redis_target::port().to_string())
            .env("RANCH_COMPAT_PREFIX", PREFIX)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn node worker.mjs");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
            log: Vec::new(),
        }
    }

    /// Next line starting with `tag`; returns its JSON payload (or `null`).
    fn expect(&mut self, tag: &str) -> serde_json::Value {
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(25))
                .unwrap_or_else(|_| {
                    panic!("no `{tag}` from the worker. Output so far: {:#?}", self.log)
                });
            self.log.push(line.clone());
            assert!(
                !line.starts_with("TIMEOUT") && !line.starts_with("WORKER_ERROR"),
                "worker reported a problem: {:#?}",
                self.log
            );
            if let Some(rest) = line.strip_prefix(tag) {
                let rest = rest.trim();
                return if rest.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::from_str(rest).expect("harness prints JSON")
                };
            }
        }
    }

    fn go(&mut self) {
        writeln!(self.stdin, "GO").expect("write GO");
        self.stdin.flush().expect("flush");
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn new_job(name: &str, data: &str) -> NewJob {
    NewJob {
        name: name.into(),
        data: data.into(),
        delay_ms: 0,
        attempts: None,
        priority: 0,
        lifo: false,
        backoff: None,
        job_id: None,
        remove_on_complete: false,
        remove_on_fail: false,
    }
}

async fn processes_what_ranch_adds(major: u8) {
    let Some(dir) = harness_or_skip() else {
        return;
    };
    let queue = format!("orders-v{major}");
    let qref = QueueRef {
        prefix: PREFIX.into(),
        name: queue.clone(),
    };
    // A previous run that was interrupted must not shift the job ids.
    {
        let mut redis = redis::Client::open(format!(
            "redis://{}:{}",
            redis_target::host(),
            redis_target::port()
        ))
        .expect("redis url")
        .get_multiplexed_async_connection()
        .await
        .expect("redis connection");
        let stale: Vec<String> = redis::cmd("KEYS")
            .arg(format!("{PREFIX}:{queue}:*"))
            .query_async(&mut redis)
            .await
            .expect("keys");
        if !stale.is_empty() {
            let _: () = redis::cmd("DEL")
                .arg(stale)
                .query_async(&mut redis)
                .await
                .expect("del");
        }
    }
    let mut worker = Harness::start(&dir, major, &queue, 5);
    let ready = worker.expect("READY");
    eprintln!("BullMQ {major}: {ready}");

    let registry = ConnectionRegistry::new();
    let (_db_dir, cfg) = local_redis_cfg().await;
    let conn_id = format!("compat-{major}");
    let payload = r#"{"orderId": 4711, "items": ["a", "b"], "note": "é \"quoted\" ✓", "nested": {"ok": true}}"#;
    let jobs = [
        ("first", new_job("first", payload)),
        (
            "low",
            NewJob {
                priority: 10,
                ..new_job("low", "{}")
            },
        ),
        (
            "high",
            NewJob {
                priority: 1,
                attempts: Some(3),
                ..new_job("high", "[1,2,3]")
            },
        ),
        (
            "later",
            NewJob {
                delay_ms: 1_500,
                ..new_job("later", "\"text\"")
            },
        ),
        (
            "urgent",
            NewJob {
                lifo: true,
                ..new_job("urgent", "null")
            },
        ),
    ];
    let mut ids = std::collections::HashMap::new();
    for (label, job) in jobs {
        let out = registry
            .add_job(&conn_id, &cfg, qref.id(), job, Caller::User)
            .await
            .expect("actor send")
            .unwrap_or_else(|e| panic!("add_job {label}: {e}"));
        let expected_state = match label {
            "low" | "high" => states::PRIORITIZED,
            "later" => JobState::Delayed,
            _ => JobState::Waiting,
        };
        assert_eq!(out.state, expected_state, "{label}");
        ids.insert(label, out.job_id);
    }
    // Auto ids, in order, like BullMQ's own counter.
    assert_eq!(ids["first"], "1");
    assert_eq!(ids["urgent"], "5");

    worker.go();

    // 1. BullMQ's own counters agree with where Ranch says it put things.
    let counts = worker.expect("COUNTS");
    assert_eq!(counts["waiting"], 2, "{counts}");
    assert_eq!(counts["prioritized"], 2, "{counts}");
    assert_eq!(counts["delayed"], 1, "{counts}");

    // 2. BullMQ's `Job.fromJSON` reads Ranch's hashes: names, options, payload.
    let parsed = worker.expect("PARSED");
    let parsed = parsed.as_array().expect("array");
    assert_eq!(parsed.len(), 5, "{parsed:?}");
    let by_name = |name: &str| {
        parsed
            .iter()
            .find(|j| j["name"] == name)
            .unwrap_or_else(|| panic!("{name} not parsed: {parsed:?}"))
    };
    let expected_payload: serde_json::Value = serde_json::from_str(payload).unwrap();
    assert_eq!(by_name("first")["data"], expected_payload);
    assert_eq!(by_name("high")["opts"]["attempts"], 3);
    assert_eq!(by_name("high")["priority"], 1);
    assert_eq!(by_name("later")["delay"], 1_500);
    assert_eq!(by_name("urgent")["data"], serde_json::Value::Null);

    // 3. The worker processes all five, in BullMQ's order: the wait list
    //    first (LIFO job ahead of the older one), then by priority, and the
    //    delayed job only once it is due.
    let mut order = Vec::new();
    let mut later_waited_ms = 0;
    for _ in 0..5 {
        let job = worker.expect("JOB");
        let name = job["name"].as_str().expect("name").to_string();
        if name == "first" {
            assert_eq!(job["data"], expected_payload, "payload round-trip");
        }
        if name == "later" {
            later_waited_ms =
                job["processedOn"].as_i64().unwrap() - job["timestamp"].as_i64().unwrap();
        }
        assert_eq!(
            job["id"],
            ids[name.as_str()].as_str(),
            "{name} keeps its id"
        );
        order.push(name);
    }
    assert_eq!(order, ["urgent", "first", "high", "low", "later"]);
    assert!(
        later_waited_ms >= 1_500,
        "the delayed job ran after {later_waited_ms} ms"
    );

    // 4. Ranch's stream entries are what QueueEvents expects.
    let events = worker.expect("EVENTS");
    let events = events.as_array().expect("array");
    let saw = |event: &str, id: &str| {
        events
            .iter()
            .any(|e| e["event"] == event && e["jobId"] == id)
    };
    for (label, id) in &ids {
        assert!(saw("added", id), "no `added` event for {label}: {events:?}");
        assert!(saw("completed", id), "no `completed` event for {label}");
    }
    assert!(saw("delayed", &ids["later"]));
    assert!(saw("waiting", &ids["first"]));

    let last = worker.expect("FINAL");
    assert_eq!(last["completed"], 5, "{last}");
    assert_eq!(last["failed"], 0, "{last}");
    worker.expect("DONE");
    registry.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Redis and the BullMQ workers: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn bullmq_5_worker_processes_what_ranch_adds() {
    processes_what_ranch_adds(5).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Redis and the BullMQ workers: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn bullmq_6_worker_processes_what_ranch_adds() {
    processes_what_ranch_adds(6).await;
}
