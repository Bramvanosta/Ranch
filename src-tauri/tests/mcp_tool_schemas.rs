//! What `tools/list` shows an agent: every tool's name, description,
//! annotations and input schema, pinned in `tests/snapshots/mcp_tools.json`.
//! The list is that of the kinds of backend `ranch-mcp` serves: BullMQ on
//! Redis, every tool.
//!
//! An agent client builds its calls from these schemas, so a change to one
//! is a change to the agent surface and should be a conscious one. After an
//! intended change, regenerate the snapshot with
//! `RANCH_UPDATE_SNAPSHOTS=1 cargo test --test mcp_tool_schemas` and commit it.

use std::path::Path;

use ranch_lib::adapters::bullmq_redis;
use ranch_lib::application::catalogue::{self, Kind, Trail};
use ranch_lib::domain::capabilities::Capability;
use ranch_lib::shells::mcp::RanchMcpServer;

const SNAPSHOT: &str = "tests/snapshots/mcp_tools.json";

#[test]
fn the_app_serves_bullmq_on_redis_alone() {
    let kinds: Vec<_> = bullmq_redis::specs()
        .iter()
        .map(|spec| spec.kind().as_str().to_string())
        .collect();
    assert_eq!(
        kinds,
        ["bullmq-redis"],
        "the in-memory kind is for tests only"
    );
}

#[test]
fn tools_list_matches_the_snapshot() {
    let mut tools = RanchMcpServer::tools_for(&bullmq_redis::specs());
    assert_eq!(
        tools.len(),
        RanchMcpServer::live_tool_names().len(),
        "BullMQ on Redis supports every tool"
    );
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    let mut current = serde_json::to_string_pretty(&tools).expect("tools serialize");
    current.push('\n');

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(SNAPSHOT);
    if std::env::var_os("RANCH_UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &current).unwrap();
        return;
    }
    let pinned = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{SNAPSHOT} is missing ({e}); see this file's header"));
    assert!(
        pinned == current,
        "the MCP tool list changed; if intended, regenerate {SNAPSHOT} (see this file's header)"
    );
}

/// With every capability declared, `tools/list` is the catalogue's tools,
/// and an annotation rmcp emits on a tool agrees with its operation's kind.
#[test]
fn tools_list_with_every_capability_is_the_catalogues_tools() {
    let specs = bullmq_redis::specs();
    for capability in Capability::ALL {
        assert!(
            specs
                .iter()
                .any(|spec| spec.capabilities().supports(capability)),
            "BullMQ on Redis does not declare {}",
            capability.name()
        );
    }
    let tools = RanchMcpServer::tools_for(&specs);
    let mut listed: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    listed.sort();
    let mut declared: Vec<&str> = catalogue::tools().map(|(name, _)| name).collect();
    declared.sort();
    assert_eq!(
        listed, declared,
        "the #[tool] methods of RanchMcpServer and the catalogue's tools differ"
    );

    for tool in &tools {
        let write = catalogue::tool(&tool.name).unwrap().kind.is_write();
        let Some(annotations) = &tool.annotations else {
            continue;
        };
        if let Some(read_only) = annotations.read_only_hint {
            assert_eq!(read_only, !write, "{}: readOnlyHint", tool.name);
        }
        if annotations.destructive_hint == Some(true) {
            assert!(write, "{}: destructiveHint on a read", tool.name);
        }
    }
}

/// The instructions name every tool served, reads and writes apart, in
/// the catalogue's order.
#[test]
fn the_instructions_list_the_tools_served() {
    let instructions = RanchMcpServer::instructions_for(&bullmq_redis::specs());
    let list = |pick: fn(Kind) -> bool| {
        catalogue::tools()
            .filter(|(_, op)| pick(op.kind))
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let reads = list(|kind| kind == Kind::Read);
    let writes = list(|kind| kind == Kind::Write(Trail::Pipeline));
    assert!(instructions.contains(&format!("READ tools: {reads};")));
    assert!(instructions.contains(&format!("in the Ranch app: {writes}.")));
    // The note on a cause has its own sentence.
    assert!(instructions.contains("attach your diagnosis with annotate_cause"));
}
