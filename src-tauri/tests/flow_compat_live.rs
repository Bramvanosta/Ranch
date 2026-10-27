//! **Ranch reads a flow made by BullMQ's own `FlowProducer`**
//! (5.x and 6.x): across two queues, two levels deep, with a completed, a
//! failed and a still-waiting branch. The flow is real — produced and
//! partly processed by real workers (`.docker/bullmq-compat/flow.mjs`) — so
//! the keys Ranch walks (`parentKey`, `:dependencies`, `:processed`,
//! `:failed` / `:unsuccessful`) are the ones BullMQ actually writes.
//!
//! ```bash
//! docker compose --profile redis up -d
//! (cd .docker/bullmq-compat && npm ci)
//! RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test flow_compat_live
//! ```

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use ranch_lib::commands::jobs::GetJobArgs;
use ranch_lib::commands::queues::QueueActionArgs;
use ranch_lib::core::bullmq_schema::{FlowTree, JobState, QueueRef};
use ranch_lib::core::connection_actor::{ConnectionConfig, ConnectionRegistry};

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

async fn local_redis_cfg() -> ConnectionConfig {
    ConnectionConfig {
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
        pool: sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory pool"),
    }
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
    fn start(dir: &PathBuf, major: u8, base: &str) -> Self {
        let mut child = Command::new("node")
            .arg("flow.mjs")
            .arg(major.to_string())
            .arg(base)
            .current_dir(dir)
            .env("RANCH_REDIS_HOST", redis_host())
            .env("RANCH_REDIS_PORT", redis_port().to_string())
            .env("RANCH_COMPAT_PREFIX", PREFIX)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn node flow.mjs");
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
                !line.starts_with("TIMEOUT"),
                "harness timed out: {:#?}",
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

fn shape(tree: &FlowTree) -> Vec<(String, Option<String>, JobState)> {
    let name_of = |key: &str| {
        tree.nodes
            .iter()
            .find(|n| n.key == key)
            .map(|n| n.name.clone())
    };
    let mut rows: Vec<_> = tree
        .nodes
        .iter()
        .map(|n| {
            (
                n.name.clone(),
                n.parent_key.as_deref().and_then(name_of),
                n.state,
            )
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

async fn reads_a_real_flow(major: u8) {
    let Some(dir) = harness_or_skip() else {
        return;
    };
    let base = format!("flow-v{major}");
    {
        let mut redis = redis::Client::open(format!("redis://{}:{}", redis_host(), redis_port()))
            .expect("redis url")
            .get_multiplexed_async_connection()
            .await
            .expect("redis connection");
        let stale: Vec<String> = redis::cmd("KEYS")
            .arg(format!("{PREFIX}:{base}-*"))
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

    let mut bullmq = Harness::start(&dir, major, &base);
    let ids = bullmq.expect("READY");
    let id = |name: &str| ids[name].as_str().expect("id").to_string();

    let registry = ConnectionRegistry::new();
    let cfg = local_redis_cfg().await;
    let conn = format!("compat-flow-{major}");
    let queue = |suffix: &str| QueueRef {
        prefix: PREFIX.into(),
        name: format!("{base}-{suffix}"),
    };
    let flow_of = |q: QueueRef, job_id: String| {
        registry.get_flow(
            &conn,
            &cfg,
            GetJobArgs {
                connection_id: conn.clone(),
                queue_ref: q,
                job_id,
            },
        )
    };

    // From the deepest job (another queue than the root's): the whole tree.
    let tree = flow_of(queue("a"), id("download"))
        .await
        .expect("actor send")
        .expect("get_flow");
    assert!(tree.is_flow());
    assert!(!tree.truncated);
    assert_eq!(tree.nodes[0].name, "assemble", "root first");
    assert_eq!(
        tree.root_key,
        format!("{PREFIX}:{base}-a:{}", id("assemble"))
    );
    assert_eq!(
        tree.focus_key,
        format!("{PREFIX}:{base}-a:{}", id("download"))
    );
    let some = |s: &str| Some(s.to_string());
    assert_eq!(
        shape(&tree),
        vec![
            ("assemble".to_string(), None, JobState::WaitingChildren),
            ("download".to_string(), some("prepare"), JobState::Completed),
            ("fetch-bad".to_string(), some("assemble"), JobState::Failed),
            (
                "fetch-ok".to_string(),
                some("assemble"),
                JobState::Completed
            ),
            ("prepare".to_string(), some("assemble"), JobState::Completed),
        ],
        "every branch, whatever became of it — across both queues"
    );
    let bad = tree.nodes.iter().find(|n| n.name == "fetch-bad").unwrap();
    assert_eq!(bad.failed_reason.as_deref(), Some("upstream 503"));
    assert_eq!(bad.job_ref.queue.name, format!("{base}-b"));
    let ok = tree.nodes.iter().find(|n| n.name == "fetch-ok").unwrap();
    assert!(ok.duration_ms.is_some());

    // Same tree from the root and from the middle; only the focus moves.
    let from_root = flow_of(queue("a"), id("assemble")).await.unwrap().unwrap();
    let from_middle = flow_of(queue("b"), id("prepare")).await.unwrap().unwrap();
    assert_eq!(shape(&from_root), shape(&tree));
    assert_eq!(shape(&from_middle), shape(&tree));
    assert_eq!(
        from_middle.focus_key,
        format!("{PREFIX}:{base}-b:{}", id("prepare"))
    );

    // Nested JSON for agents.
    let nested = tree.to_nested_json();
    assert_eq!(nested["root"]["name"], "assemble");
    assert_eq!(nested["root"]["state"], "waitingChildren");
    assert_eq!(nested["root"]["children"].as_array().unwrap().len(), 3);

    // The queue's Flows tab: roots only — `prepare` has children but also a parent.
    let roots_a = registry
        .list_flow_roots(
            &conn,
            &cfg,
            QueueActionArgs {
                connection_id: conn.clone(),
                queue_ref: queue("a"),
            },
        )
        .await
        .expect("actor send")
        .expect("list_flow_roots");
    assert_eq!(roots_a.roots.len(), 1, "{roots_a:?}");
    let root = &roots_a.roots[0];
    assert_eq!(root.name, "assemble");
    assert_eq!(root.state, JobState::WaitingChildren);
    assert_eq!(
        (
            root.children_total,
            root.children_completed,
            root.children_failed,
            root.children_pending
        ),
        (3, 2, 1, 0),
        "BullMQ {major}: {root:?}"
    );
    assert!(!roots_a.capped);
    let roots_b = registry
        .list_flow_roots(
            &conn,
            &cfg,
            QueueActionArgs {
                connection_id: conn.clone(),
                queue_ref: queue("b"),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(
        roots_b.roots.is_empty(),
        "no root lives in queue b: {roots_b:?}"
    );

    // A job that is in no flow: a tree of one.
    let err = flow_of(queue("a"), "nope".into())
        .await
        .unwrap()
        .expect_err("missing job");
    assert!(err.to_string().contains("not found"), "{err}");

    bullmq.go();
    bullmq.expect("DONE");
    registry.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_a_flow_made_by_bullmq_5() {
    reads_a_real_flow(5).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_a_flow_made_by_bullmq_6() {
    reads_a_real_flow(6).await;
}
