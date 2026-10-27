//! Workers attached to a queue.
//!
//! BullMQ has no worker registry: `Queue.getWorkers()` reads `CLIENT LIST`
//! and keeps the connections whose **name** is the queue's client name —
//! `{prefix}:{base64(queue name)}` for an unnamed worker, that plus
//! `:w:{worker name}` for a named one (the worker's blocking connection is
//! the one that carries it). This module is that logic, pure.
//!
//! Managed Redis often forbids `CLIENT` (GCP Memorystore, some ElastiCache /
//! Upstash plans, ACL users without `+client`). That is a capability of the
//! provider, not a failure: [`client_list_denied`] recognises the refusal so
//! the actor can answer `supported: false` once and stop asking.

use std::collections::BTreeMap;

use base64::Engine;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::domain::QueueRef;

/// One BullMQ worker = one named Redis connection.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueueWorker {
    /// Redis client id (`id=`), unique per connection on that server.
    pub id: String,
    /// The worker's own name (`new Worker(q, fn, { name })`), if it has one.
    pub name: Option<String>,
    /// `ip:port` of the worker process as Redis sees it.
    pub addr: String,
    /// Seconds since the connection was opened.
    pub age_secs: u64,
    /// Seconds since its last command. A healthy idle worker sits in a
    /// blocking pop, so this stays under its `drainDelay` (5 s by default).
    pub idle_secs: u64,
    /// Last command (`cmd=`), e.g. `bzpopmin`.
    pub last_command: Option<String>,
    /// Client library as reported by Redis ≥ 7.2 (`lib-name` / `lib-ver`).
    pub library: Option<String>,
    /// Every `key=value` of the `CLIENT LIST` line, for the detail sheet.
    pub raw: BTreeMap<String, String>,
}

/// Result of `list_workers`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueueWorkers {
    /// `false` = this Redis refuses `CLIENT LIST`; `workers` is empty and
    /// `unsupported_reason` carries the server's message.
    pub supported: bool,
    pub unsupported_reason: Option<String>,
    pub workers: Vec<QueueWorker>,
}

impl QueueWorkers {
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self {
            supported: false,
            unsupported_reason: Some(reason.into()),
            workers: Vec::new(),
        }
    }
}

/// How many workers one queue has.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueueWorkerCount {
    pub queue_ref: QueueRef,
    pub workers: u32,
}

/// Result of `count_queue_workers`: one `CLIENT LIST` for many queues.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueueWorkerCounts {
    /// `false` = this Redis refuses `CLIENT LIST`: `counts` is empty, and no
    /// queue may be called worker-less from it.
    pub supported: bool,
    pub counts: Vec<QueueWorkerCount>,
}

/// The number of workers of each queue in `refs`, read from one
/// raw `CLIENT LIST` reply (same matching as [`workers_for_queue`]).
pub fn count_workers(client_list: &str, refs: &[QueueRef]) -> Vec<QueueWorkerCount> {
    let names: Vec<String> = client_list
        .lines()
        .filter_map(|line| parse_line(line.trim()).remove("name"))
        .filter(|name| !name.is_empty())
        .collect();
    refs.iter()
        .map(|qref| {
            let unnamed = queue_client_name(qref);
            let named_prefix = format!("{unnamed}:w:");
            let workers = names
                .iter()
                .filter(|n| **n == unnamed || n.starts_with(&named_prefix))
                .count();
            QueueWorkerCount {
                queue_ref: qref.clone(),
                workers: u32::try_from(workers).unwrap_or(u32::MAX),
            }
        })
        .collect()
}

/// `{prefix}:{base64(name)}` — BullMQ's `QueueBase.clientName()`.
pub fn queue_client_name(qref: &QueueRef) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(qref.name.as_bytes());
    format!("{}:{}", qref.prefix, encoded)
}

/// Parse one `CLIENT LIST` line into its `key=value` pairs. Values never
/// contain spaces (Redis forbids them in client names), so splitting on
/// spaces is exact.
fn parse_line(line: &str) -> BTreeMap<String, String> {
    line.split(' ')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (!key.is_empty()).then(|| (key.to_string(), value.to_string()))
        })
        .collect()
}

/// Workers of `qref` found in a raw `CLIENT LIST` reply, oldest first.
pub fn workers_for_queue(client_list: &str, qref: &QueueRef) -> Vec<QueueWorker> {
    let unnamed = queue_client_name(qref);
    let named_prefix = format!("{unnamed}:w:");
    let mut workers: Vec<QueueWorker> = client_list
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let raw = parse_line(line);
            let client_name = raw.get("name")?;
            let name = if *client_name == unnamed {
                None
            } else {
                // `:qe` (QueueEvents) and other suffixes are not workers.
                Some(client_name.strip_prefix(&named_prefix)?.to_string())
            };
            let seconds = |key: &str| {
                raw.get(key)
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0)
            };
            let text = |key: &str| raw.get(key).filter(|v| !v.is_empty()).cloned();
            let library = match (text("lib-name"), text("lib-ver")) {
                (Some(n), Some(v)) => Some(format!("{n} {v}")),
                (Some(n), None) => Some(n),
                _ => None,
            };
            Some(QueueWorker {
                id: raw.get("id").cloned().unwrap_or_default(),
                name,
                addr: raw.get("addr").cloned().unwrap_or_default(),
                age_secs: seconds("age"),
                idle_secs: seconds("idle"),
                last_command: text("cmd").filter(|c| c != "NULL"),
                library,
                raw,
            })
        })
        .collect();
    workers.sort_by(|a, b| b.age_secs.cmp(&a.age_secs).then_with(|| a.id.cmp(&b.id)));
    workers
}

/// Is this Redis error the provider saying "`CLIENT LIST` is not for you"?
/// (unknown / renamed command, ACL `NOPERM`, managed-service refusals.)
pub fn client_list_denied(message: &str) -> bool {
    let m = message.to_lowercase();
    m.contains("unknown command")
        || m.contains("noperm")
        || m.contains("no permissions")
        || m.contains("not allowed")
        || m.contains("not supported")
        || m.contains("command is disabled")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emails() -> QueueRef {
        QueueRef {
            prefix: "bull".into(),
            name: "emails".into(),
        }
    }

    // Shape of a real Redis 7.2 reply (trimmed), CRLF-free like redis-rs returns it.
    const CLIENT_LIST: &str = "\
id=12 addr=172.18.0.5:51234 laddr=172.18.0.2:6379 fd=9 name=bull:ZW1haWxz age=3600 idle=2 flags=b db=0 sub=0 psub=0 multi=-1 qbuf=0 cmd=bzpopmin user=default lib-name=ioredis lib-ver=5.4.1
id=13 addr=172.18.0.5:51236 laddr=172.18.0.2:6379 fd=10 name=bull:ZW1haWxz:w:mailer-eu age=120 idle=1 flags=b db=0 cmd=bzpopmin user=default lib-name= lib-ver=
id=14 addr=172.18.0.5:51240 fd=11 name=bull:ZW1haWxz:qe age=50 idle=0 flags=b db=0 cmd=xread
id=15 addr=172.18.0.6:40000 fd=12 name=bull:aW1hZ2Vz age=10 idle=0 flags=b db=0 cmd=bzpopmin
id=16 addr=172.18.0.7:40001 fd=13 name= age=5 idle=0 flags=N db=0 cmd=client|list
";

    #[test]
    fn client_name_is_prefix_and_base64_of_the_queue_name() {
        assert_eq!(queue_client_name(&emails()), "bull:ZW1haWxz");
        let odd = QueueRef {
            prefix: "{app}".into(),
            name: "émails 📬".into(),
        };
        assert_eq!(
            queue_client_name(&odd),
            format!(
                "{{app}}:{}",
                base64::engine::general_purpose::STANDARD.encode("émails 📬")
            )
        );
    }

    #[test]
    fn keeps_this_queues_workers_named_or_not_oldest_first() {
        let workers = workers_for_queue(CLIENT_LIST, &emails());
        assert_eq!(workers.len(), 2, "not QueueEvents, not images, not Ranch");

        assert_eq!(workers[0].id, "12");
        assert_eq!(workers[0].name, None);
        assert_eq!(workers[0].addr, "172.18.0.5:51234");
        assert_eq!(workers[0].age_secs, 3600);
        assert_eq!(workers[0].idle_secs, 2);
        assert_eq!(workers[0].last_command.as_deref(), Some("bzpopmin"));
        assert_eq!(workers[0].library.as_deref(), Some("ioredis 5.4.1"));
        assert_eq!(
            workers[0].raw.get("user").map(String::as_str),
            Some("default")
        );

        assert_eq!(workers[1].name.as_deref(), Some("mailer-eu"));
        assert_eq!(workers[1].library, None, "empty lib-name is no library");
    }

    #[test]
    fn a_queue_whose_name_prefixes_another_does_not_steal_its_workers() {
        // base64("email") = "ZW1haWw=" — not a prefix of "ZW1haWxz", and the
        // match is exact-or-`:w:` anyway.
        let email = QueueRef {
            prefix: "bull".into(),
            name: "email".into(),
        };
        assert!(workers_for_queue(CLIENT_LIST, &email).is_empty());
        assert!(workers_for_queue("", &emails()).is_empty());
        assert!(workers_for_queue("garbage without pairs\n\n", &emails()).is_empty());
    }

    #[test]
    fn recognises_a_provider_refusal() {
        for msg in [
            "ERR unknown command 'CLIENT', with args beginning with: 'LIST'",
            "ERR unknown command `client`",
            "NOPERM this user has no permissions to run the 'client|list' command",
            "NOPERM User default has no permissions to run the 'client' command",
            "ERR Command is not allowed",
            "ERR command is not supported",
            "ERR CLIENT command is disabled",
        ] {
            assert!(client_list_denied(msg), "{msg}");
        }
        for msg in [
            "Connection refused (os error 61)",
            "timed out",
            "WRONGPASS invalid password",
        ] {
            assert!(!client_list_denied(msg), "{msg}");
        }
    }

    #[test]
    fn counts_workers_of_many_queues_from_one_client_list() {
        let images = QueueRef {
            prefix: "bull".into(),
            name: "images".into(),
        };
        let reports = QueueRef {
            prefix: "bull".into(),
            name: "reports".into(),
        };
        let counts = count_workers(CLIENT_LIST, &[emails(), images.clone(), reports.clone()]);
        // emails: the unnamed and the named worker, not the QueueEvents (`:qe`).
        assert_eq!(counts[0].workers, 2);
        assert_eq!(
            counts[1],
            QueueWorkerCount {
                queue_ref: images,
                workers: 1
            }
        );
        assert_eq!(
            counts[2],
            QueueWorkerCount {
                queue_ref: reports,
                workers: 0
            }
        );
    }

    #[test]
    fn counts_agree_with_the_worker_list() {
        let counts = count_workers(CLIENT_LIST, &[emails()]);
        assert_eq!(
            counts[0].workers as usize,
            workers_for_queue(CLIENT_LIST, &emails()).len()
        );
    }
}
