//! **the queue stays consistent for real BullMQ** after Ranch
//! retries all failed jobs, drains and cleans. For BullMQ 5.x
//! and 6.x, `.docker/bullmq-compat/maintenance.mjs` produces real failed and
//! completed jobs with a real `Worker`; Ranch operates on them; BullMQ's own
//! API then says what it sees — and a worker re-processes what was retried.
//!
//! ```bash
//! docker compose --profile redis up -d
//! (cd .docker/bullmq-compat && npm ci)
//! RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test maintenance_compat_live
//! ```
//!
//! Skipped without `RANCH_LOCAL_REDIS`; with `RANCH_BULLMQ_COMPAT=1` (CI) a
//! missing Node harness is a failure, not a skip.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use ranch_lib::commands::queues::QueueMaintenanceArgs;
use ranch_lib::core::bullmq_schema::{CleanState, QueueMaintenance, QueueRef};
use ranch_lib::core::connection_actor::{AuditAttribution, ConnectionConfig, ConnectionRegistry};

const PREFIX: &str = "ranchcompat";

fn redis_port() -> u16 {
    std::env::var("RANCH_REDIS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6390)
}

fn redis_host() -> String {
    std::env::var("RANCH_REDIS_HOST").unwrap_or_else(|_| "127.0.0.1".into())
}

async fn local_redis_cfg() -> (tempfile::TempDir, ConnectionConfig) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = ranch_lib::core::db::Db::init(dir.path()).await.expect("db");
    let cfg = ConnectionConfig {
        host: redis_host(),
        port: redis_port(),
        db: 0,
        username: None,
        password: None,
        tls: false,
        tls_custom_ca: None,
        ssh_tunnel: None,
        ssh_password: None,
        keychain: std::sync::Arc::new(ranch_lib::core::keychain::InMemoryKeychain::new()),
        initial_lock_state: ranch_lib::core::safety_lock::ReadOnlyState::Unlocked,
        connection_id: "compat-conn".into(),
        connection_name: "compat".into(),
        env_color: Some("env-local".into()),
        pool: db.pool().clone(),
    };
    (dir, cfg)
}

fn harness_or_skip() -> Option<PathBuf> {
    if std::env::var("RANCH_LOCAL_REDIS").is_err() {
        eprintln!("skipping: set RANCH_LOCAL_REDIS=1 + `docker compose --profile redis up -d`");
        return None;
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.docker/bullmq-compat");
    let installed =
        dir.join("node_modules/bullmq5").exists() && dir.join("node_modules/bullmq6").exists();
    let node = Command::new("node").arg("--version").output().is_ok();
    if installed && node {
        return Some(dir);
    }
    let why = "BullMQ compat harness missing: run `npm ci` in .docker/bullmq-compat (needs Node)";
    assert!(std::env::var("RANCH_BULLMQ_COMPAT").is_err(), "{why}");
    eprintln!("skipping: {why}");
    None
}

struct Harness {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    log: Vec<String>,
}

impl Harness {
    fn start(dir: &PathBuf, major: u8, queue: &str) -> Self {
        let mut child = Command::new("node")
            .arg("maintenance.mjs")
            .arg(major.to_string())
            .arg(queue)
            .current_dir(dir)
            .env("RANCH_REDIS_HOST", redis_host())
            .env("RANCH_REDIS_PORT", redis_port().to_string())
            .env("RANCH_COMPAT_PREFIX", PREFIX)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn node maintenance.mjs");
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

    fn expect(&mut self, tag: &str) -> serde_json::Value {
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(35))
                .unwrap_or_else(|_| panic!("no `{tag}` from the harness: {:#?}", self.log));
            self.log.push(line.clone());
            assert!(
                !line.starts_with("TIMEOUT") && !line.starts_with("WORKER_ERROR"),
                "harness reported a problem: {:#?}",
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

async fn stays_consistent(major: u8) {
    let Some(dir) = harness_or_skip() else {
        return;
    };
    let queue = format!("maintenance-v{major}");
    {
        let mut redis = redis::Client::open(format!("redis://{}:{}", redis_host(), redis_port()))
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

    let mut bullmq = Harness::start(&dir, major, &queue);
    let registry = ConnectionRegistry::new();
    let (_db_dir, cfg) = local_redis_cfg().await;
    let conn_id = format!("compat-maint-{major}");
    let run = |operation: QueueMaintenance| {
        let args = QueueMaintenanceArgs {
            connection_id: conn_id.clone(),
            queue_ref: QueueRef {
                prefix: PREFIX.into(),
                name: queue.clone(),
            },
            operation,
        };
        registry.run_queue_maintenance(&conn_id, &cfg, args, None, AuditAttribution::ui())
    };

    // ── Real failures, produced by a real worker ─────────────────────────
    let stage_a = bullmq.expect("STAGE_A");
    assert_eq!(stage_a["failed"], 6, "{stage_a}");
    assert_eq!(stage_a["completed"], 3, "{stage_a}");

    let retried = run(QueueMaintenance::RetryAllFailed)
        .await
        .expect("actor send")
        .expect("retry all failed");
    assert_eq!((retried.affected, retried.skipped), (6, 0));

    bullmq.go();
    let after_retry = bullmq.expect("AFTER_RETRY");
    assert_eq!(after_retry["failed"], 0, "{after_retry}");
    assert_eq!(after_retry["waiting"], 6, "{after_retry}");

    // A real worker processes what Ranch retried — and BullMQ kept counting
    // the attempts, so the second run succeeds.
    let reprocessed = bullmq.expect("REPROCESSED");
    assert_eq!(reprocessed["counts"]["completed"], 9, "{reprocessed}");
    assert_eq!(reprocessed["counts"]["failed"], 0, "{reprocessed}");
    let reruns = reprocessed["reruns"].as_array().expect("reruns");
    assert_eq!(reruns.len(), 6);
    assert!(
        reruns
            .iter()
            .all(|r| r["name"] == "flaky" && r["attemptsMade"] == 1),
        "{reruns:?}"
    );

    // ── Drain + clean ────────────────────────────────────────────────────
    let stage_b = bullmq.expect("STAGE_B");
    // An `every` scheduler's first run is due now: it sits in `wait`.
    assert_eq!(
        stage_b["waiting"], 5,
        "4 jobs + the scheduler's run: {stage_b}"
    );
    assert_eq!(stage_b["prioritized"], 1, "{stage_b}");
    assert_eq!(stage_b["delayed"], 2, "{stage_b}");

    let drained = run(QueueMaintenance::Drain {
        include_delayed: true,
    })
    .await
    .expect("actor send")
    .expect("drain");
    assert_eq!(
        (drained.affected, drained.skipped),
        (7, 1),
        "everything but the scheduler's pending run"
    );
    let cleaned = run(QueueMaintenance::Clean {
        state: CleanState::Completed,
        older_than_ms: 0,
    })
    .await
    .expect("actor send")
    .expect("clean");
    assert_eq!((cleaned.affected, cleaned.skipped), (9, 0));

    bullmq.go();
    let after = bullmq.expect("AFTER_CLEANUP");
    let counts = &after["counts"];
    for state in ["prioritized", "completed", "failed", "active", "delayed"] {
        assert_eq!(counts[state], 0, "{state}: {counts}");
    }
    assert_eq!(
        counts["waiting"], 1,
        "the schedule survives a drain: {counts}"
    );
    assert_eq!(
        after["schedulers"].as_array().map(Vec::len),
        Some(1),
        "{after}"
    );

    // No orphan: the only job hash left is the scheduler's pending run.
    let keys: Vec<String> = {
        let mut redis = redis::Client::open(format!("redis://{}:{}", redis_host(), redis_port()))
            .expect("redis url")
            .get_multiplexed_async_connection()
            .await
            .expect("redis connection");
        redis::cmd("KEYS")
            .arg(format!("{PREFIX}:{queue}:*"))
            .query_async(&mut redis)
            .await
            .expect("keys")
    };
    let base = format!("{PREFIX}:{queue}:");
    let structural = [
        "meta",
        "id",
        "events",
        "marker",
        "wait",
        "repeat",
        "pc",
        "stalled-check",
        "de",
    ];
    let job_keys: Vec<&str> = keys
        .iter()
        .map(String::as_str)
        .filter(|k| {
            let rest = k.strip_prefix(&base).unwrap_or(k);
            !structural.contains(&rest) && !rest.starts_with("repeat:")
        })
        .collect();
    assert!(
        job_keys.is_empty(),
        "orphan keys: {job_keys:?} (all: {keys:?})"
    );

    bullmq.go();
    bullmq.expect("DONE");
    registry.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bullmq_5_queue_stays_consistent_after_retry_all_drain_and_clean() {
    stays_consistent(5).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bullmq_6_queue_stays_consistent_after_retry_all_drain_and_clean() {
    stays_consistent(6).await;
}
