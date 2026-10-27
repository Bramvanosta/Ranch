//! Integration test enforcing the UI/MCP parity contract.
//!
//! **The contract is asymmetric: `UI ⊇ MCP`.**
//! Every MCP tool must have a matching UI command (modulo the documented
//! aliases like `inspect_job` ↔ `get_job`). UI-only commands (form helpers,
//! lock toggles, admin operations) stay UI-only by design.

use ranch_lib::commands::registry;
use std::collections::HashSet;

#[test]
fn mcp_is_a_subset_of_ui_after_alias_resolution() {
    let ui: HashSet<&str> = registry::ui_command_names().into_iter().collect();
    let mcp: HashSet<&str> = registry::mcp_tool_names().into_iter().collect();

    // Assert MCP ⊆ UI (after resolving aliases). Each MCP tool
    // name must either (a) be present verbatim in the UI set, OR (b) alias
    // to a UI command via `mcp_alias_to_ui`.
    let mcp_unmatched: Vec<&str> = mcp
        .iter()
        .filter(|t| {
            let resolved = registry::mcp_alias_to_ui(t);
            !ui.contains(resolved)
        })
        .copied()
        .collect();

    assert!(
        mcp_unmatched.is_empty(),
        "UI/MCP parity contract violated. MCP tools without a matching UI command: {:?}",
        mcp_unmatched
    );

    // Sanity checks on the asymmetric counts (UI ⊃ MCP). The UI count is every
    // command Ranch registers. The MCP count is the agent-facing subset: the
    // read tools, plus the write tools that go through the safety gate. The
    // rest is UI-only on purpose (installers, settings, approval and lock
    // controls, windows, form helpers) or MCP-forbidden (anything that would
    // let an agent loosen its own limits or read its own approval trail).
    assert_eq!(
        ui.len(),
        96,
        "expected 96 UI commands, found {}: {:?}",
        ui.len(),
        ui
    );
    assert_eq!(
        mcp.len(),
        29,
        "expected 29 MCP tools, found {}: {:?}",
        mcp.len(),
        mcp
    );
}

/// Explicitly enumerate the MCP surface so any addition / removal forces a
/// conscious update of this test (+ the narrative in `commands::registry`).
#[test]
fn mcp_surface_is_exactly_the_declared_tools() {
    let mcp: HashSet<&str> = registry::mcp_tool_names().into_iter().collect();
    let expected: HashSet<&str> = [
        // read
        "list_connections",
        "list_environments",
        "list_queues",
        "list_jobs",
        "inspect_job",
        "get_job_logs",
        "list_schedulers",
        "get_queue_metrics",
        "get_connection_metrics",
        "list_failed_jobs",
        "list_workers",
        "get_flow",
        "list_failure_causes",
        // write
        "add_job",
        "annotate_cause",
        "drain_queue",
        "clean_queue",
        "upsert_scheduler",
        "remove_scheduler",
        "retry_job",
        "remove_job",
        "promote_job",
        "bulk_retry_jobs",
        "bulk_remove_jobs",
        "bulk_promote_jobs",
        "pause_queue",
        "resume_queue",
        "pause_scheduler",
        "resume_scheduler",
    ]
    .into_iter()
    .collect();
    assert_eq!(mcp, expected, "MCP tool set drifted from the declared list");
}
