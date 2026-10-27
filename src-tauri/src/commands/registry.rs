//! The central command registry — the spine of Ranch's UI/MCP parity contract.
//!
//! **The parity contract is asymmetric: `UI ⊇ MCP`.**
//! The MCP surface is the set of tools defined in `core::mcp`;
//! UI-only commands (form helpers like `parse_redis_url`, admin ops like
//! `add_connection` / `update_connection` / `unlock_connection`) are
//! deliberately not on the MCP side because they're either non-sensical
//! for an agent or actively dangerous (the lock toggle must remain a
//! human-only affordance).
//!
//! The parity test (`tests/parity_contract.rs`) asserts:
//!   - Every MCP tool has a matching UI command (subset).
//!   - The asymmetric counts: 35 UI commands, 8 MCP tools.

use crate::core::mcp::gate::ToolKind;

/// Canonical UI command name list — single source of truth for the Tauri
/// invoke handler. Adding a Tauri command: register here AND in
/// `commands/mod.rs::specta_builder`. The parity-contract integration test
/// catches drift on the MCP side.
const COMMAND_NAMES: &[&str] = &[
    // Connection wizard surface.
    "parse_redis_url",
    "test_connection",
    "add_connection",
    "list_connections",
    // Sidebar reachability + inline rename + cross-restart prefs.
    "ping_connection",
    "rename_environment",
    "get_kv_setting",
    "set_kv_setting",
    // Manage connections (edit / duplicate / remove / change color / reorder).
    "update_connection",
    "duplicate_connection",
    "remove_connection",
    // Recoloring an environment (the color is an attribute of the environment).
    "change_environment_color",
    "reorder_groups",
    "reorder_environments",
    // Queue list with live counters.
    "list_queues",
    // Paginated job list per (queue, state) — first JobDecoder caller.
    "list_jobs",
    // Job drill-down (single fetch + BullMQ logs read).
    "get_job",
    "get_job_logs",
    // Failed jobs across the queues of a connection.
    "list_failed_jobs",
    // Failed jobs grouped by cause (UI-only for now).
    "list_failure_causes",
    // Recent activity (overview pages) — UI-only: an agent asks `list_jobs`.
    "list_recent_jobs",
    // Enqueue a job.
    "add_job",
    // CRON / repeatable jobs view.
    "list_cron_jobs",
    // Workers attached to a queue.
    "list_workers",
    // Flows: the tree of a job, the roots of a queue.
    "get_flow",
    "list_flow_roots",
    // Job schedulers (the run preview is pure and UI-only).
    "preview_schedule",
    // Runs of a schedule in a window, for the 24 h strip (UI-only).
    "schedule_runs_within",
    "upsert_scheduler",
    "remove_scheduler",
    // Queue maintenance (preview is read-only and UI-only: an
    // agent's preview is the approval modal).
    "preview_queue_maintenance",
    "drain_queue",
    "clean_queue",
    "retry_all_failed",
    // Audit log read + export.
    "list_audit_log_entries",
    "export_audit_log",
    // per-connection read-only lock + unlock UX.
    "get_connection_lock_state",
    "unlock_connection",
    "lock_connection",
    // single-job destructive actions (retry / remove / promote).
    "retry_job",
    "remove_job",
    "promote_job",
    // Bulk single-queue destructive actions.
    "bulk_retry_jobs",
    "bulk_remove_jobs",
    "bulk_promote_jobs",
    // Pause / resume queue + CRON scheduler.
    "pause_queue",
    "resume_queue",
    "pause_scheduler",
    "resume_scheduler",
    // Direct env enumeration (MCP-driven; also UI-callable).
    "list_environments",
    // MCP installer commands (UI-only by design).
    "get_mcp_installer_status",
    "install_mcp_for_claude_desktop",
    "install_mcp_for_cursor",
    // Generic installer (UI-only: an agent must not wire itself
    // into another agent's config).
    "install_mcp_for_client",
    "get_mcp_manual_snippet",
    // Auto-update orchestration (UI-only).
    "check_for_updates",
    // Downloads, verifies, replaces the app and restarts it — on the user's
    // click. UI-only: an agent has no business restarting the app.
    "install_update",
    // 24h observability dashboards (UI-only — agent path
    // covers data access via list_queues + list_jobs already).
    "get_queue_metrics",
    "get_connection_metrics",
    // Throughput of every queue, for sparklines and heat strips (UI-only).
    "get_throughput",
    // The Timeline's dragged window (UI-only).
    "summarize_window",
    // The Map tab's flow links and rates (UI-only).
    "get_topology",
    // Live updates from BullMQ's event streams (UI-only: it
    // sends events to the app's windows).
    "follow_connection_events",
    // The Redis server for the overview's info strip (UI-only).
    "get_server_info",
    // Live events of the overview (UI-only).
    "read_queue_events",
    // What needs the user across connections (UI-only).
    "list_attention",
    // Groups as first-class entities (UI-only: organising the
    // sidebar is a human affordance, and the delete cascade removes
    // keychain entries).
    "list_groups",
    "create_group",
    "rename_group",
    "delete_group",
    "set_group_collapsed",
    // Sidebar folding and its worker counts (UI-only: sidebar
    // state, and agents have `list_workers` per queue).
    "set_connection_collapsed",
    "count_queue_workers",
    // Native windows and tabs (UI-only, desktop-only).
    "open_window",
    // The Settings window (UI-only, desktop-only).
    "open_settings_window",
    // Environments as entities + moving connections (UI-only).
    "create_environment",
    "delete_environment",
    "move_connection",
    // MCP safety settings (UI-only AND MCP-forbidden).
    "get_mcp_safety_settings",
    "set_mcp_safety_settings",
    // The AI features switch (UI-only, MCP-forbidden).
    "set_ai_features",
    "set_connection_mcp_readonly",
    // The approval queue, GUI side (UI-only AND MCP-forbidden).
    "list_pending_approvals",
    "decide_pending_approval",
    // The approval behind an audited write (UI-only AND MCP-forbidden).
    "get_approval_record",
    // Earlier approval decisions (UI-only, MCP-forbidden).
    "list_approval_history",
    // MCP activity log, GUI side (UI-only).
    "list_mcp_activity",
    "list_mcp_activity_sessions",
    "export_mcp_activity",
    "clear_mcp_activity",
    // Resolution notes (annotate_cause is also an MCP tool).
    "annotate_cause",
    "list_cause_notes",
    "dismiss_cause_note",
    // Ask AI (UI-only, MCP-forbidden).
    "launch_claude_code",
    "open_agent_app",
    "share_for_agents",
];

/// Canonical MCP tool list — every agent-facing tool with its static
/// classification. Each tool's body delegates to the matching Tauri `*_inner`
/// function so behaviour stays unified between the two surfaces.
/// `inspect_job` is the agent-facing alias for `get_job`.
///
/// **The classification is the contract.** `Read` tools are never
/// gated. `Write` tools go through the MCP safety gate (`core::mcp::gate`):
/// MCP read-only → human approval → session lock. There is no runtime
/// heuristic; adding a tool means declaring its kind here, and the parity
/// test fails on a tool the server exposes without an entry.
///
/// The parity test also enforces that every name here resolves to a UI
/// command (`UI ⊇ MCP`, aliases allow-listed).
const MCP_TOOLS: &[(&str, ToolKind)] = &[
    ("list_connections", ToolKind::Read),
    ("list_environments", ToolKind::Read),
    ("list_queues", ToolKind::Read),
    ("list_jobs", ToolKind::Read),
    ("inspect_job", ToolKind::Read), // alias of `get_job`
    // More to read.
    ("get_job_logs", ToolKind::Read),
    ("list_schedulers", ToolKind::Read), // alias of `list_cron_jobs`
    ("get_queue_metrics", ToolKind::Read),
    ("get_connection_metrics", ToolKind::Read),
    // "what failed on staging?" in one call.
    ("list_failed_jobs", ToolKind::Read),
    // "is anything consuming this queue?"
    ("list_workers", ToolKind::Read),
    // "why is this parent stuck in waiting-children?"
    ("get_flow", ToolKind::Read),
    // The causes to annotate, as the UI groups them.
    ("list_failure_causes", ToolKind::Read),
    // Writes — share the lock-aware actor handlers with the UI.
    ("retry_job", ToolKind::Write),
    ("remove_job", ToolKind::Write),
    ("promote_job", ToolKind::Write),
    // One call = one approval, so bulk exists on this surface too.
    ("bulk_retry_jobs", ToolKind::Write),
    ("bulk_remove_jobs", ToolKind::Write),
    ("bulk_promote_jobs", ToolKind::Write),
    ("pause_queue", ToolKind::Write),
    ("resume_queue", ToolKind::Write),
    ("pause_scheduler", ToolKind::Write),
    ("resume_scheduler", ToolKind::Write),
    // The approval preview shows queue, name, options, payload.
    ("add_job", ToolKind::Write),
    // Queue maintenance; the preview states the affected count.
    // ("retry all failed" is `bulk_retry_jobs` with `allFailed: true`.)
    // A note on a cause. Write for the read-only flag, but no
    // approval: it changes no queue state (handled in the tool itself).
    ("annotate_cause", ToolKind::Write),
    ("drain_queue", ToolKind::Write),
    ("clean_queue", ToolKind::Write),
    // The preview shows the schedule, the next run, the template.
    ("upsert_scheduler", ToolKind::Write),
    ("remove_scheduler", ToolKind::Write),
];

/// Commands that must **never** become MCP tools: anything that
/// loosens a safety control. If an agent could call these it could lift the
/// very restrictions that apply to it. Enforced by the parity test, which
/// walks the live MCP tool router as well as `MCP_TOOLS`.
const MCP_FORBIDDEN_COMMANDS: &[&str] = &[
    // The human session lock.
    "unlock_connection",
    "lock_connection",
    // MCP read-only flags and the global safety settings.
    "set_connection_mcp_readonly",
    "set_mcp_safety_settings",
    // The AI features switch (UI-only, MCP-forbidden).
    "set_ai_features",
    // An agent must never see, approve or deny a request,
    // its own least of all.
    "list_pending_approvals",
    "decide_pending_approval",
    "get_approval_record",
    // Earlier approval decisions (UI-only, MCP-forbidden).
    "list_approval_history",
    // Generic key/value writer: `mcp_readonly_default` and the approval
    // settings live in `kv_settings`.
    "set_kv_setting",
    // An agent must not erase its own trail, nor read the
    // sessions of other agents through Ranch.
    "clear_mcp_activity",
    "list_mcp_activity",
    "list_mcp_activity_sessions",
    "export_mcp_activity",
    // An agent must not wire itself (or Ranch) into another agent's config.
    "install_mcp_for_client",
    "install_mcp_for_claude_desktop",
    "install_mcp_for_cursor",
    // An agent must not hide notes, its own or others'.
    "dismiss_cause_note",
    // An agent must never start processes on the user's machine.
    "launch_claude_code",
    "open_agent_app",
    // Destroys keychain entries.
    "delete_group",
    "remove_connection",
];

/// Aliases from agent-facing MCP names → UI command names. Lets the parity
/// test resolve `inspect_job` (MCP) ↔ `get_job` (UI) without faking entries
/// in either list.
const MCP_ALIASES: &[(&str, &str)] = &[
    ("inspect_job", "get_job"),
    // BullMQ calls them job schedulers; the UI command predates that.
    ("list_schedulers", "list_cron_jobs"),
];

/// Names of all commands exposed to the Tauri UI layer.
pub fn ui_command_names() -> Vec<&'static str> {
    COMMAND_NAMES.to_vec()
}

/// Names of all tools exposed to the MCP server.
pub fn mcp_tool_names() -> Vec<&'static str> {
    MCP_TOOLS.iter().map(|(name, _)| *name).collect()
}

/// Static read / write classification of an MCP tool. `None` = unknown tool.
pub fn mcp_tool_kind(name: &str) -> Option<ToolKind> {
    MCP_TOOLS
        .iter()
        .find_map(|(tool, kind)| (*tool == name).then_some(*kind))
}

/// Commands that must never be exposed as MCP tools.
pub fn mcp_forbidden_commands() -> Vec<&'static str> {
    MCP_FORBIDDEN_COMMANDS.to_vec()
}

/// MCP-name → UI-name resolution table. Returns the UI command name that
/// backs the given MCP tool. Used by the parity test + by future audit
/// rows to attribute the call to the right command.
pub fn mcp_alias_to_ui(mcp_name: &str) -> &str {
    MCP_ALIASES
        .iter()
        .find_map(|(alias, target)| (*alias == mcp_name).then_some(*target))
        .unwrap_or(mcp_name)
}
