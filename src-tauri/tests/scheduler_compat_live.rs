//! **a real BullMQ worker receives the jobs of a scheduler made
//! in Ranch, at the expected times**. A scheduler is only as
//! good as its pending job: the worker that finishes it computes and creates
//! the next one from `opts.repeat`. So this test lets real BullMQ 5.x and 6.x
//! workers run schedulers Ranch created, edited, paused, resumed and removed.
//!
//! ```bash
//! docker compose --profile redis up -d
//! (cd .docker/bullmq-compat && npm ci)
//! RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test scheduler_compat_live
//! ```
//!
//! ~12 s per BullMQ major (it waits for real ticks). Skipped without
//! `RANCH_LOCAL_REDIS`; with `RANCH_BULLMQ_COMPAT=1` a missing harness fails.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use ranch_lib::commands::queues::{SchedulerActionArgs, UpsertSchedulerArgs};
use ranch_lib::core::bullmq_schema::{QueueRef, Schedule, SchedulerSpec};
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
            .arg("scheduler.mjs")
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
            .expect("spawn node scheduler.mjs");
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
                .recv_timeout(Duration::from_secs(45))
                .unwrap_or_else(|_| panic!("no `{tag}` from the harness: {:#?}", self.log));
            self.log.push(line.clone());
            assert!(
                !line.starts_with("TIMEOUT") && !line.starts_with("WORKER_ERROR"),
                "harness reported a problem: {:#?}",
                self.log
            );
            if let Some(rest) = line.strip_prefix(tag) {
                let rest = rest.trim();
                if rest.is_empty() {
                    return serde_json::Value::Null;
                }
                // `RUNS_A` is also a prefix of nothing else; `FINAL`/`DONE` are exact.
                return serde_json::from_str(rest).expect("harness prints JSON");
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

/// Runs of one scheduler in a stage, as `(at ms, data)`.
fn runs_of(stage: &serde_json::Value, scheduler: &str) -> Vec<(i64, serde_json::Value)> {
    stage
        .as_array()
        .expect("runs")
        .iter()
        .filter(|r| r["scheduler"] == scheduler)
        .map(|r| (r["at"].as_i64().expect("at"), r["data"].clone()))
        .collect()
}

fn gaps(runs: &[(i64, serde_json::Value)]) -> Vec<i64> {
    runs.windows(2).map(|w| w[1].0 - w[0].0).collect()
}

async fn worker_receives_scheduled_jobs(major: u8) {
    let Some(dir) = harness_or_skip() else {
        return;
    };
    let queue = format!("scheduler-v{major}");
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
    bullmq.expect("READY");

    let registry = ConnectionRegistry::new();
    let (_db_dir, cfg) = local_redis_cfg().await;
    let conn_id = format!("compat-sched-{major}");
    let qref = QueueRef {
        prefix: PREFIX.into(),
        name: queue.clone(),
    };
    let upsert = |id: &str, job_name: &str, schedule: Schedule, data: &str| {
        let args = UpsertSchedulerArgs {
            connection_id: conn_id.clone(),
            queue_ref: qref.clone(),
            scheduler: SchedulerSpec {
                id: id.into(),
                job_name: job_name.into(),
                schedule,
                data: data.into(),
            },
        };
        registry.upsert_scheduler(&conn_id, &cfg, args, AuditAttribution::ui())
    };
    let action = |id: &str| SchedulerActionArgs {
        connection_id: conn_id.clone(),
        queue_ref: qref.clone(),
        scheduler_key: id.into(),
    };

    // ── Create: an interval and a cron pattern (6 fields: every second) ──
    let tick = upsert(
        "tick",
        "heartbeat",
        Schedule::Every { every_ms: 1_000 },
        r#"{"v":1}"#,
    )
    .await
    .expect("actor send")
    .expect("upsert tick");
    assert!(tick.job_id.starts_with("repeat:tick:"), "{tick:?}");
    upsert(
        "cron",
        "cron-beat",
        Schedule::Pattern {
            pattern: "* * * * * *".into(),
            tz: Some("Europe/Paris".into()),
        },
        r#"{"kind":"cron"}"#,
    )
    .await
    .expect("actor send")
    .expect("upsert cron");

    bullmq.go();
    // BullMQ's own API reads what Ranch wrote.
    let schedulers = bullmq.expect("SCHEDULERS");
    let find = |list: &serde_json::Value, id: &str| {
        list.as_array()
            .expect("array")
            .iter()
            .find(|s| s["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} not listed by BullMQ: {list}"))
    };
    let t = find(&schedulers, "tick");
    assert_eq!(t["every"], 1_000, "{t}");
    assert_eq!(t["name"], "heartbeat", "{t}");
    assert_eq!(t["data"], serde_json::json!({"v": 1}), "{t}");
    let c = find(&schedulers, "cron");
    assert_eq!(c["pattern"], "* * * * * *", "{c}");
    assert_eq!(c["tz"], "Europe/Paris", "{c}");

    // A real worker receives the jobs — and keeps the chain going: only the
    // first job of each scheduler was created by Ranch.
    let a = bullmq.expect("RUNS_A");
    let tick_a = runs_of(&a, "tick");
    let cron_a = runs_of(&a, "cron");
    assert!(
        tick_a.len() >= 3,
        "tick ran {} times in 3.5 s: {a}",
        tick_a.len()
    );
    assert!(
        cron_a.len() >= 3,
        "cron ran {} times in 3.5 s: {a}",
        cron_a.len()
    );
    assert!(tick_a
        .iter()
        .all(|(_, d)| *d == serde_json::json!({"v": 1})));
    assert!(cron_a
        .iter()
        .all(|(_, d)| *d == serde_json::json!({"kind": "cron"})));
    for gap in gaps(&tick_a) {
        assert!(
            (600..=1_500).contains(&gap),
            "every 1000 ms, got a gap of {gap} ms"
        );
    }

    // ── Edit the interval + template; pause the cron ─────────────────────
    upsert(
        "tick",
        "heartbeat",
        Schedule::Every { every_ms: 2_000 },
        r#"{"v":2}"#,
    )
    .await
    .expect("actor send")
    .expect("edit tick");
    registry
        .pause_scheduler(&conn_id, &cfg, action("cron"))
        .await
        .expect("actor send")
        .expect("pause cron");
    // Let jobs that were already picked up finish before measuring.
    tokio::time::sleep(Duration::from_millis(300)).await;
    bullmq.go();
    let b = bullmq.expect("RUNS_B");
    let tick_b = runs_of(&b, "tick");
    let edited: Vec<_> = tick_b
        .iter()
        .filter(|(_, d)| *d == serde_json::json!({"v": 2}))
        .cloned()
        .collect();
    assert!(edited.len() >= 2, "the edited scheduler runs: {b}");
    for gap in gaps(&edited) {
        assert!(
            (1_500..=2_600).contains(&gap),
            "every 2000 ms, got a gap of {gap} ms"
        );
    }
    let cron_b = runs_of(&b, "cron");
    assert!(
        cron_b.len() <= 1,
        "a paused scheduler produces nothing: {b}"
    );

    // ── Resume the cron; remove the interval ─────────────────────────────
    registry
        .resume_scheduler(&conn_id, &cfg, action("cron"))
        .await
        .expect("actor send")
        .expect("resume cron");
    registry
        .remove_scheduler(&conn_id, &cfg, action("tick"), AuditAttribution::ui())
        .await
        .expect("actor send")
        .expect("remove tick");
    tokio::time::sleep(Duration::from_millis(300)).await;
    bullmq.go();
    let c = bullmq.expect("RUNS_C");
    assert!(
        runs_of(&c, "cron").len() >= 3,
        "resumed: fires every second again: {c}"
    );
    assert!(
        runs_of(&c, "tick").len() <= 1,
        "removed: no further job: {c}"
    );

    let last = bullmq.expect("FINAL");
    let ids: Vec<&str> = last
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|s| s["id"].as_str())
        .collect();
    assert_eq!(ids, ["cron"], "BullMQ lists what Ranch left: {last}");

    bullmq.expect("DONE");
    registry.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bullmq_5_worker_receives_the_jobs_of_a_scheduler_made_in_ranch() {
    worker_receives_scheduled_jobs(5).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bullmq_6_worker_receives_the_jobs_of_a_scheduler_made_in_ranch() {
    worker_receives_scheduled_jobs(6).await;
}
