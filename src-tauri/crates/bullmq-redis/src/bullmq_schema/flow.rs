//! Flows: parent / child job graphs.
//!
//! BullMQ's `FlowProducer` links jobs through their hashes and four side
//! keys, possibly across queues:
//!
//!   - child → parent: the child's `parentKey` field (`{prefix}:{queue}:{id}`);
//!   - parent → children: `{job}:dependencies` (set: children still
//!     pending), `{job}:processed` (hash child key → return value),
//!     `{job}:failed` (hash: failed children it was told to ignore) and
//!     `{job}:unsuccessful` (zset: failed children that fail the parent).
//!
//! A parent waits in `waiting-children` until `dependencies` is empty.
//!
//! This module is the pure half: key parsing, the (flat) tree model, its
//! nested JSON form for agents, and the bounds.

use super::domain::{states, JobRef, JobState, QueueRef};
pub use ranch_domain::flows::{FlowNode, FlowRoot, FlowRoots, FlowTree};

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
/// can prefixes (`{app}:bull`), hence `known_prefix`: a flow normally lives
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
    Some(
        QueueRef {
            prefix,
            name: queue.to_string(),
        }
        .job(id),
    )
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
    parent == states::WAITING_CHILDREN
        && children
            .iter()
            .any(|(state, blocks)| *state == JobState::Failed && *blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jref(prefix: &str, queue: &str, id: &str) -> JobRef {
        QueueRef {
            prefix: prefix.into(),
            name: queue.into(),
        }
        .job(id)
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
            "so may the prefix, when it is the known one"
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
            states::WAITING_CHILDREN,
            &[(Completed, true), (Failed, true)]
        ));
        assert!(
            !is_blocked(states::WAITING_CHILDREN, &[(Failed, false)]),
            "the child lets it go on"
        );
        assert!(!is_blocked(
            states::WAITING_CHILDREN,
            &[(Active, true), (Completed, true)]
        ));
        assert!(
            !is_blocked(Failed, &[(Failed, true)]),
            "failed parent: not waiting"
        );
    }
}
