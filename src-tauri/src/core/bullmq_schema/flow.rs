//! Flows — parent / child job graphs.
//!
//! BullMQ's `FlowProducer` links jobs through their hashes and four side
//! keys, possibly across queues:
//!
//!   - child → parent: the child's `parentKey` field (`{prefix}:{queue}:{id}`);
//!   - parent → children: `{job}:dependencies` (set — children still
//!     pending), `{job}:processed` (hash child key → return value),
//!     `{job}:failed` (hash — failed children it was told to ignore) and
//!     `{job}:unsuccessful` (zset — failed children that fail the parent).
//!
//! A parent waits in `waiting-children` until `dependencies` is empty.
//!
//! This module is the pure half: key parsing, the (flat) tree model, its
//! nested JSON form for agents, and the bounds.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::domain::{JobRef, JobState, QueueRef};

/// A flow is cut off beyond this many jobs (and says so).
pub const MAX_NODES: usize = 200;
/// Parents followed upwards from the job the user is looking at.
pub const MAX_ANCESTORS: usize = 25;
/// Flow discovery in a queue: parents waiting for children…
pub const DISCOVERY_WAITING_CHILDREN: usize = 500;
/// …plus the most recently finished jobs, per state.
pub const DISCOVERY_RECENT_PER_STATE: usize = 250;

/// Split a job key into prefix / queue / id. Queue names cannot contain `:`
/// (BullMQ refuses them) but job ids can (`repeat:nightly:1789…`) and so
/// can prefixes (`{app}:bull`) — hence `known_prefix`: a flow normally lives
/// under one prefix, the one of the queue the user came from.
pub fn parse_job_key(key: &str, known_prefix: &str) -> Option<JobRef> {
    let with_known = key
        .strip_prefix(known_prefix)
        .and_then(|rest| rest.strip_prefix(':'))
        .map(|rest| (known_prefix.to_string(), rest));
    let (prefix, rest) = match with_known {
        Some(found) => found,
        // Another prefix: take its first segment.
        None => {
            let (prefix, rest) = key.split_once(':')?;
            (prefix.to_string(), rest)
        }
    };
    let (queue, id) = rest.split_once(':')?;
    if queue.is_empty() || id.is_empty() {
        return None;
    }
    Some(JobRef {
        queue: QueueRef {
            prefix,
            name: queue.to_string(),
        },
        id: id.to_string(),
    })
}

/// One job of a flow. The tree is flat — `parent_key` links the nodes — so
/// it crosses the IPC boundary without recursive types and feeds a graph
/// layout directly.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowNode {
    /// `{prefix}:{queue}:{id}` — unique within the flow.
    pub key: String,
    pub job_ref: JobRef,
    /// `None` for the root.
    pub parent_key: Option<String>,
    pub name: String,
    pub state: JobState,
    pub timestamp: Option<i64>,
    /// `finishedOn − processedOn`, when both are known.
    pub duration_ms: Option<i64>,
    pub failed_reason: Option<String>,
    /// The job hash no longer exists (removed by `removeOnComplete`, a clean…):
    /// the parent still references it.
    pub missing: bool,
    /// What a finished job returned (raw JSON), if anything.
    #[serde(default)]
    pub return_value: Option<String>,
    /// This job failed for good and its parent waits for it
    /// forever (no `failParentOnFailure` / `ignoreDependencyOnFailure`…).
    #[serde(default)]
    pub blocks_parent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowTree {
    /// Root first, then breadth-first.
    pub nodes: Vec<FlowNode>,
    pub root_key: String,
    /// The job the flow was asked for.
    pub focus_key: String,
    /// The flow holds more than [`MAX_NODES`] jobs; the rest is not shown.
    pub truncated: bool,
    /// A parent waits on a child that failed for good.
    #[serde(default)]
    pub blocked: bool,
}

impl FlowTree {
    /// A job on its own is not a flow.
    pub fn is_flow(&self) -> bool {
        self.nodes.len() > 1
    }

    /// The tree as nested JSON — what an agent reads (`get_flow`).
    pub fn to_nested_json(&self) -> serde_json::Value {
        let mut children: HashMap<&str, Vec<&FlowNode>> = HashMap::new();
        for node in &self.nodes {
            if let Some(parent) = node.parent_key.as_deref() {
                children.entry(parent).or_default().push(node);
            }
        }
        fn build(node: &FlowNode, children: &HashMap<&str, Vec<&FlowNode>>) -> serde_json::Value {
            let kids: Vec<serde_json::Value> = children
                .get(node.key.as_str())
                .map(|list| list.iter().map(|c| build(c, children)).collect())
                .unwrap_or_default();
            serde_json::json!({
                "queue": node.job_ref.queue.name,
                "jobId": node.job_ref.id,
                "name": node.name,
                "state": node.state,
                "durationMs": node.duration_ms,
                "failedReason": node.failed_reason,
                "missing": node.missing,
                "children": kids,
            })
        }
        let root = self.nodes.iter().find(|n| n.key == self.root_key);
        serde_json::json!({
            "focus": self.focus_key,
            "truncated": self.truncated,
            "jobCount": self.nodes.len(),
            "root": root.map(|r| build(r, &children)),
        })
    }
}

/// Does a failed child hold its parent forever? BullMQ
/// moves the parent on only when the child says so in its options:
/// `failParentOnFailure` fails it, `ignoreDependencyOnFailure`,
/// `removeDependencyOnFailure` and `continueParentOnFailure` let it go on.
/// Without any of them the failed child stays a dependency and the parent
/// waits in `waiting-children` for good.
pub fn failure_blocks_parent(opts_json: Option<&str>) -> bool {
    let Some(opts) = opts_json.and_then(|o| serde_json::from_str::<serde_json::Value>(o).ok())
    else {
        return true;
    };
    ![
        "failParentOnFailure",
        "ignoreDependencyOnFailure",
        "removeDependencyOnFailure",
        "continueParentOnFailure",
    ]
    .iter()
    .any(|k| opts.get(k).and_then(serde_json::Value::as_bool) == Some(true))
}

/// A parent is blocked when it waits for its children and one of them failed
/// for good without letting it go on.
pub fn is_blocked(parent: JobState, children: &[(JobState, bool)]) -> bool {
    parent == JobState::WaitingChildren
        && children
            .iter()
            .any(|(state, blocks)| *state == JobState::Failed && *blocks)
}

/// A flow root found in a queue (the queue's Flows tab).
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowRoot {
    pub job_ref: JobRef,
    pub name: String,
    pub state: JobState,
    pub timestamp: Option<i64>,
    /// Direct children, by what became of them.
    pub children_total: u64,
    pub children_pending: u64,
    pub children_completed: u64,
    pub children_failed: u64,
    /// The root waits on a child that failed for good.
    #[serde(default)]
    pub blocked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowRoots {
    /// Newest first.
    pub roots: Vec<FlowRoot>,
    /// Jobs looked at.
    pub scanned: u64,
    /// The queue holds more finished jobs than discovery looks at: older
    /// flows are not listed.
    pub capped: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jref(prefix: &str, queue: &str, id: &str) -> JobRef {
        JobRef {
            queue: QueueRef {
                prefix: prefix.into(),
                name: queue.into(),
            },
            id: id.into(),
        }
    }

    #[test]
    fn parses_job_keys_including_ids_and_prefixes_with_colons() {
        assert_eq!(
            parse_job_key("bull:emails:42", "bull"),
            Some(jref("bull", "emails", "42"))
        );
        assert_eq!(
            parse_job_key("bull:emails:repeat:nightly:1789", "bull"),
            Some(jref("bull", "emails", "repeat:nightly:1789")),
            "an id may contain ':'"
        );
        assert_eq!(
            parse_job_key("{app}:bull:emails:42", "{app}:bull"),
            Some(jref("{app}:bull", "emails", "42")),
            "so may the prefix — when it is the known one"
        );
        assert_eq!(
            parse_job_key("other:images:7", "bull"),
            Some(jref("other", "images", "7")),
            "a child under another prefix"
        );
        // `bullet:…` must not be read as prefix `bull` + queue `et`.
        assert_eq!(
            parse_job_key("bullet:q:1", "bull"),
            Some(jref("bullet", "q", "1"))
        );
        for bad in ["", "bull", "bull:emails", "bull:emails:", "bull::1"] {
            assert_eq!(parse_job_key(bad, "bull"), None, "{bad}");
        }
    }

    fn node(key: &str, parent: Option<&str>, state: JobState) -> FlowNode {
        let job_ref = parse_job_key(key, "bull").unwrap();
        FlowNode {
            key: key.into(),
            name: format!("job-{}", job_ref.id),
            job_ref,
            parent_key: parent.map(Into::into),
            state,
            timestamp: Some(1),
            duration_ms: None,
            failed_reason: None,
            missing: false,
            return_value: None,
            blocks_parent: false,
        }
    }

    #[test]
    fn nests_the_flat_tree_for_agents() {
        let tree = FlowTree {
            nodes: vec![
                node("bull:a:1", None, JobState::WaitingChildren),
                node("bull:b:2", Some("bull:a:1"), JobState::Completed),
                node("bull:b:3", Some("bull:a:1"), JobState::Failed),
                node("bull:a:4", Some("bull:b:3"), JobState::Waiting),
            ],
            root_key: "bull:a:1".into(),
            focus_key: "bull:b:3".into(),
            truncated: false,
            blocked: false,
        };
        assert!(tree.is_flow());
        let json = tree.to_nested_json();
        assert_eq!(json["jobCount"], 4);
        assert_eq!(json["focus"], "bull:b:3");
        assert_eq!(json["root"]["jobId"], "1");
        assert_eq!(json["root"]["state"], "waitingChildren");
        let kids = json["root"]["children"].as_array().unwrap();
        assert_eq!(kids.len(), 2);
        assert_eq!(kids[1]["queue"], "b");
        assert_eq!(kids[1]["children"][0]["jobId"], "4");
        assert_eq!(kids[0]["children"].as_array().unwrap().len(), 0);

        let alone = FlowTree {
            nodes: vec![node("bull:a:1", None, JobState::Waiting)],
            root_key: "bull:a:1".into(),
            focus_key: "bull:a:1".into(),
            truncated: false,
            blocked: false,
        };
        assert!(!alone.is_flow());
    }

    // A parent stuck behind a failed child.
    #[test]
    fn a_failed_child_blocks_its_parent_unless_its_options_say_otherwise() {
        assert!(failure_blocks_parent(Some(r#"{"attempts":3}"#)));
        assert!(failure_blocks_parent(None));
        assert!(failure_blocks_parent(Some("not json")));
        for k in [
            "failParentOnFailure",
            "ignoreDependencyOnFailure",
            "removeDependencyOnFailure",
            "continueParentOnFailure",
        ] {
            assert!(
                !failure_blocks_parent(Some(&format!(r#"{{"{k}":true}}"#))),
                "{k}"
            );
        }
        assert!(failure_blocks_parent(Some(
            r#"{"failParentOnFailure":false}"#
        )));

        use JobState::*;
        assert!(is_blocked(
            WaitingChildren,
            &[(Completed, true), (Failed, true)]
        ));
        assert!(
            !is_blocked(WaitingChildren, &[(Failed, false)]),
            "the child lets it go on"
        );
        assert!(!is_blocked(
            WaitingChildren,
            &[(Active, true), (Completed, true)]
        ));
        assert!(
            !is_blocked(Failed, &[(Failed, true)]),
            "failed parent: not waiting"
        );
    }
}
