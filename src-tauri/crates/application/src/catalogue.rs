//! Every operation Ranch offers, once: its name, whether it reads or
//! writes, the capability it needs of a connection's backend, and which
//! shells expose it.
//!
//! The desktop app exposes every operation: an operation is a UI command
//! first (`UI ⊇ MCP`, `UI ⊇ server`). The HTTP server serves it or refuses
//! it with a reason; the MCP server lists it as a tool (under its own name
//! or another), leaves it to the UI with a reason, or forbids it, because
//! an agent could use it to lift its own limits or reach outside Ranch.
//!
//! Plain data. The shells keep the literal lists their macros need
//! (`collect_commands!`, the `#[tool]` methods, the HTTP routes), and a
//! test of each shell fails when its list and this one differ.

use ranch_domain::capabilities::Capability::{self, *};

/// One operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operation {
    /// The command name: the Tauri command, the HTTP route.
    pub name: &'static str,
    pub kind: Kind,
    /// What the operation requires of a connection's backend: a backend
    /// that does not declare it answers `unsupported`, and the MCP tool
    /// list leaves out a tool no kind served declares it for.
    pub needs: Option<Capability>,
    pub server: Server,
    pub mcp: Mcp,
}

/// Whether an operation changes anything. The MCP safety gate holds every
/// write tool (agent read-only, approval); reads pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Changes nothing Ranch or a backend keeps.
    Read,
    Write(Trail),
}

/// What answers for a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trail {
    /// The write pipeline: the connection's lock, or one human approval,
    /// authorizes it, and the audit log records its outcome.
    Pipeline,
    /// Outside the write pipeline, and why.
    Outside(&'static str),
}

/// What the HTTP server does with the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Server {
    Served,
    /// Refused with `desktopOnly` and this message.
    Refused(&'static str),
}

/// What the MCP server does with the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mcp {
    /// A tool of the command's name.
    Tool,
    /// A tool of another name.
    ToolNamed(&'static str),
    /// Not a tool, and why.
    UiOnly(&'static str),
    /// Never a tool, and why.
    Forbidden(&'static str),
}

impl Kind {
    pub fn is_write(self) -> bool {
        matches!(self, Kind::Write(_))
    }
}

impl Operation {
    /// The MCP tool's name, when the operation is one.
    pub fn tool(&self) -> Option<&'static str> {
        match self.mcp {
            Mcp::Tool => Some(self.name),
            Mcp::ToolNamed(name) => Some(name),
            Mcp::UiOnly(_) | Mcp::Forbidden(_) => None,
        }
    }

    pub fn served(&self) -> bool {
        self.server == Server::Served
    }
}

/// The operation behind a command name.
pub fn operation(name: &str) -> Option<&'static Operation> {
    OPERATIONS.iter().find(|op| op.name == name)
}

/// The operation behind an MCP tool name.
pub fn tool(name: &str) -> Option<&'static Operation> {
    OPERATIONS.iter().find(|op| op.tool() == Some(name))
}

/// Every MCP tool, in the catalogue's order, with its operation.
pub fn tools() -> impl Iterator<Item = (&'static str, &'static Operation)> {
    OPERATIONS
        .iter()
        .filter_map(|op| op.tool().map(|name| (name, op)))
}

// The writes, through the write pipeline or outside it and why.
const PIPELINE: Kind = Write(Pipeline);
const RECORD: Kind = Write(Outside(
    "Ranch's own records, not a connection's queues: no lock, no audit row",
));
const LOCK: Kind = Write(Outside("the lock itself: audited, never refused by it"));
const NOTE: Kind = Write(Outside(
    "a note Ranch keeps on a cause changes no queue state: the lock does not hold it and the \
     audit log does not record it; an agent's note is still refused on a connection read-only \
     for agents",
));
const EXTERNAL: Kind = Write(Outside(
    "acts outside Ranch's records and the connections' queues",
));

// Why the HTTP server refuses a command.
const WINDOWS: &str = "Windows and tabs are the browser's in the server image.";
const IMAGE_UPDATE: &str = "The server image does not update itself: pull a newer tag.";
const MCP_SETUP: &str = "Agents (MCP) are set up from the Ranch desktop app: the MCP server \
                         runs next to the agent, on your machine.";

// Why an operation is not an MCP tool.
const FORM: &str = "a helper of a form in the app";
const ORGANISE: &str = "organising connections, groups and environments is the user's";
const SCREEN: &str = "what a screen of the app shows; agents read queues, jobs and metrics \
                      through their own tools";
const PREFERENCES: &str = "the app's own preferences";
const AGENT_LIMITS: &str = "an agent must not loosen the limits that apply to it";
const APPROVALS: &str = "an agent must never see, approve or deny a request, its own least of all";
const ACTIVITY: &str = "an agent must not read the sessions of other agents through Ranch";
const INSTALLERS: &str = "an agent must not wire itself (or Ranch) into another agent's config";
const PROCESSES: &str = "an agent must never start processes on the user's machine";
const KEYCHAIN: &str = "destroys keychain entries";
const USERS_LOCK: &str = "the lock is the user's control";
const AGENT_SETTINGS: &str = "the settings that govern agents, shown to the user";
const AUDIT: &str = "the user's record of who changed what";
const APP: &str = "the app's own windows and updates";

use Kind::{Read, Write};
use Mcp::{Forbidden, Tool, ToolNamed, UiOnly};
use Server::{Refused, Served};
use Trail::{Outside, Pipeline};

/// An operation being declared: its MCP exposure ends the declaration.
struct Draft(Operation);

const fn op(name: &'static str, kind: Kind) -> Draft {
    Draft(Operation {
        name,
        kind,
        needs: None,
        server: Served,
        mcp: Tool,
    })
}

impl Draft {
    const fn needs(mut self, capability: Capability) -> Self {
        self.0.needs = Some(capability);
        self
    }

    const fn refused(mut self, why: &'static str) -> Self {
        self.0.server = Refused(why);
        self
    }

    const fn tool(self) -> Operation {
        self.0
    }

    const fn tool_named(mut self, name: &'static str) -> Operation {
        self.0.mcp = ToolNamed(name);
        self.0
    }

    const fn ui_only(mut self, why: &'static str) -> Operation {
        self.0.mcp = UiOnly(why);
        self.0
    }

    const fn forbidden(mut self, why: &'static str) -> Operation {
        self.0.mcp = Forbidden(why);
        self.0
    }
}

/// Every operation. The order is the MCP server's when it lists its read
/// and write tools to an agent.
pub const OPERATIONS: &[Operation] = &[
    // ── connections, groups, environments ──────────────────────────────
    op("parse_redis_url", Read).ui_only(FORM),
    op("test_connection", Read).ui_only(FORM),
    op("add_connection", RECORD).ui_only(ORGANISE),
    op("list_connections", Read).tool(),
    // The kinds of backend a connection may name, and their forms.
    op("list_backends", Read).ui_only(FORM),
    op("ping_connection", Read).ui_only(SCREEN),
    op("list_environments", Read).tool(),
    op("rename_environment", RECORD).ui_only(ORGANISE),
    op("update_connection", RECORD).ui_only(ORGANISE),
    op("duplicate_connection", RECORD).ui_only(ORGANISE),
    op("remove_connection", RECORD).forbidden(KEYCHAIN),
    op("change_environment_color", RECORD).ui_only(ORGANISE),
    op("reorder_groups", RECORD).ui_only(ORGANISE),
    op("reorder_environments", RECORD).ui_only(ORGANISE),
    op("list_groups", Read).ui_only(ORGANISE),
    op("create_group", RECORD).ui_only(ORGANISE),
    op("rename_group", RECORD).ui_only(ORGANISE),
    // With `cascade`, its connections and their keychain entries go too.
    op("delete_group", RECORD).forbidden(KEYCHAIN),
    op("set_group_collapsed", RECORD).ui_only(ORGANISE),
    op("set_connection_collapsed", RECORD).ui_only(ORGANISE),
    op("create_environment", RECORD).ui_only(ORGANISE),
    op("delete_environment", RECORD).ui_only(ORGANISE),
    op("move_connection", RECORD).ui_only(ORGANISE),
    // ── settings ───────────────────────────────────────────────────────
    op("get_kv_setting", Read).ui_only(PREFERENCES),
    // The agent safety settings live in the same table.
    op("set_kv_setting", RECORD).forbidden(AGENT_LIMITS),
    // ── queues and jobs, read ──────────────────────────────────────────
    op("list_queues", Read).tool(),
    op("list_jobs", Read).tool(),
    op("get_job", Read).tool_named("inspect_job"),
    op("get_job_logs", Read).tool(),
    // BullMQ calls them job schedulers.
    op("list_cron_jobs", Read)
        .needs(SchedulesList)
        .tool_named("list_schedulers"),
    op("preview_schedule", Read).ui_only(FORM),
    // The runs of a schedule in a window, for the 24 h strip.
    op("schedule_runs_within", Read).ui_only(SCREEN),
    op("get_flow", Read).needs(Flows).tool(),
    op("list_flow_roots", Read).needs(Flows).ui_only(SCREEN),
    op("list_workers", Read).needs(WorkersList).tool(),
    // The sidebar's worker counts; agents have `list_workers` per queue.
    op("count_queue_workers", Read)
        .needs(WorkersCount)
        .ui_only(SCREEN),
    op("list_failed_jobs", Read).tool(),
    op("list_failure_causes", Read).tool(),
    // The overview's recent activity; an agent asks `list_jobs`.
    op("list_recent_jobs", Read).ui_only(SCREEN),
    op("get_queue_metrics", Read).needs(MetricsQueue).tool(),
    op("get_connection_metrics", Read)
        .needs(MetricsQueue)
        .tool(),
    op("get_throughput", Read)
        .needs(MetricsThroughput)
        .ui_only(SCREEN),
    op("summarize_window", Read)
        .needs(MetricsTimeline)
        .ui_only(SCREEN),
    op("get_topology", Read)
        .needs(ServerTopology)
        .ui_only(SCREEN),
    op("get_server_info", Read)
        .needs(ServerInfo)
        .ui_only(SCREEN),
    op("read_queue_events", Read)
        .needs(EventsRead)
        .ui_only(SCREEN),
    // A backend that cannot follow is not followed, and the app polls:
    // nothing is refused. It sends the events to the app's windows.
    op("follow_connection_events", Read).ui_only(SCREEN),
    op("list_attention", Read).ui_only(SCREEN),
    // ── queues and jobs, write ─────────────────────────────────────────
    op("retry_job", PIPELINE).needs(JobActionsSingle).tool(),
    op("remove_job", PIPELINE).needs(JobActionsSingle).tool(),
    op("promote_job", PIPELINE).needs(JobActionsSingle).tool(),
    // One call = one approval, so agents have the bulk actions too.
    op("bulk_retry_jobs", PIPELINE).needs(JobActionsBulk).tool(),
    op("bulk_remove_jobs", PIPELINE)
        .needs(JobActionsBulk)
        .tool(),
    op("bulk_promote_jobs", PIPELINE)
        .needs(JobActionsBulk)
        .tool(),
    op("add_job", PIPELINE).needs(JobActionsAdd).tool(),
    // Needs what the maintenance it previews needs.
    op("preview_queue_maintenance", Read).ui_only("an agent's preview is the approval sheet"),
    op("drain_queue", PIPELINE).needs(MaintenanceDrain).tool(),
    op("clean_queue", PIPELINE).needs(MaintenanceClean).tool(),
    op("retry_all_failed", PIPELINE)
        .needs(MaintenanceRetryAllFailed)
        .ui_only("agents retry every failed job with `bulk_retry_jobs` and `allFailed`"),
    op("pause_queue", PIPELINE).needs(QueueControl).tool(),
    op("resume_queue", PIPELINE).needs(QueueControl).tool(),
    op("pause_scheduler", PIPELINE).needs(SchedulesPause).tool(),
    op("resume_scheduler", PIPELINE)
        .needs(SchedulesPause)
        .tool(),
    op("upsert_scheduler", PIPELINE)
        .needs(SchedulesWrite)
        .tool(),
    op("remove_scheduler", PIPELINE)
        .needs(SchedulesWrite)
        .tool(),
    // ── cause notes ────────────────────────────────────────────────────
    op("annotate_cause", NOTE).tool(),
    op("list_cause_notes", Read).ui_only(SCREEN),
    op("dismiss_cause_note", NOTE).forbidden("an agent must not hide notes, its own or others'"),
    // ── the lock and the audit log ─────────────────────────────────────
    op("get_connection_lock_state", Read).ui_only(USERS_LOCK),
    op("unlock_connection", LOCK).forbidden(USERS_LOCK),
    op("lock_connection", LOCK).forbidden(USERS_LOCK),
    op("list_audit_log_entries", Read).ui_only(AUDIT),
    op("export_audit_log", Read).ui_only(AUDIT),
    // ── agents: safety, approvals, activity ────────────────────────────
    op("get_mcp_safety_settings", Read).ui_only(AGENT_SETTINGS),
    op("set_mcp_safety_settings", RECORD).forbidden(AGENT_LIMITS),
    op("set_ai_features", RECORD)
        .refused(MCP_SETUP)
        .forbidden(AGENT_LIMITS),
    op("set_connection_mcp_readonly", RECORD).forbidden(AGENT_LIMITS),
    // Served so that the shared front end, which polls the approval
    // queue, gets an empty list rather than an error.
    op("list_pending_approvals", Read).forbidden(APPROVALS),
    op("decide_pending_approval", RECORD).forbidden(APPROVALS),
    // The approval behind an audited write.
    op("get_approval_record", Read).forbidden(APPROVALS),
    op("list_approval_history", Read).forbidden(APPROVALS),
    op("list_mcp_activity", Read).forbidden(ACTIVITY),
    op("list_mcp_activity_sessions", Read).forbidden(ACTIVITY),
    op("export_mcp_activity", Read).forbidden(ACTIVITY),
    op("clear_mcp_activity", RECORD).forbidden("an agent must not erase its own trail"),
    // ── agents: setup and Ask AI ───────────────────────────────────────
    op("get_mcp_installer_status", Read)
        .refused(MCP_SETUP)
        .ui_only(INSTALLERS),
    op("install_mcp_for_claude_desktop", EXTERNAL)
        .refused(MCP_SETUP)
        .forbidden(INSTALLERS),
    op("install_mcp_for_cursor", EXTERNAL)
        .refused(MCP_SETUP)
        .forbidden(INSTALLERS),
    op("install_mcp_for_client", EXTERNAL)
        .refused(MCP_SETUP)
        .forbidden(INSTALLERS),
    op("get_mcp_manual_snippet", Read)
        .refused(MCP_SETUP)
        .ui_only(INSTALLERS),
    op("launch_claude_code", EXTERNAL)
        .refused(MCP_SETUP)
        .forbidden(PROCESSES),
    op("open_agent_app", EXTERNAL)
        .refused(MCP_SETUP)
        .forbidden(PROCESSES),
    op("share_for_agents", Read)
        .refused(MCP_SETUP)
        .ui_only("masks job data for an Ask AI prompt as MCP masks it for agents"),
    // ── the app ────────────────────────────────────────────────────────
    op("open_window", Read).refused(WINDOWS).ui_only(APP),
    op("open_settings_window", Read)
        .refused(WINDOWS)
        .ui_only(APP),
    op("check_for_updates", Read).ui_only(APP),
    // Downloads, verifies and replaces the app, then restarts it, on the
    // user's click.
    op("install_update", EXTERNAL)
        .refused(IMAGE_UPDATE)
        .ui_only(APP),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_are_unique_across_commands_and_tools() {
        let mut seen = HashSet::new();
        for op in OPERATIONS {
            assert!(seen.insert(op.name), "`{}` is listed twice", op.name);
        }
        for (name, op) in tools() {
            // A tool takes its command's name, or one no command has.
            assert!(
                name == op.name || operation(name).is_none(),
                "the tool `{name}` of `{}` is another command's name",
                op.name
            );
        }
        let tools: Vec<_> = tools().map(|(name, _)| name).collect();
        let unique: HashSet<_> = tools.iter().collect();
        assert_eq!(unique.len(), tools.len(), "a tool name is used twice");
    }

    #[test]
    fn every_reason_says_something() {
        for op in OPERATIONS {
            let reasons = [
                match op.kind {
                    Kind::Write(Trail::Outside(why)) => Some(why),
                    _ => None,
                },
                match op.server {
                    Server::Refused(why) => Some(why),
                    Server::Served => None,
                },
                match op.mcp {
                    Mcp::UiOnly(why) | Mcp::Forbidden(why) => Some(why),
                    Mcp::Tool | Mcp::ToolNamed(_) => None,
                },
            ];
            for why in reasons.into_iter().flatten() {
                assert!(
                    !why.trim().is_empty(),
                    "`{}` gives an empty reason",
                    op.name
                );
            }
        }
    }

    #[test]
    fn lookups_find_commands_and_tools() {
        assert_eq!(
            operation("get_job").and_then(Operation::tool),
            Some("inspect_job")
        );
        assert_eq!(tool("inspect_job").map(|op| op.name), Some("get_job"));
        assert_eq!(tool("get_job"), None);
        assert_eq!(operation("inspect_job"), None);
        assert_eq!(
            tool("list_schedulers").and_then(|op| op.needs),
            Some(SchedulesList)
        );
    }
}
