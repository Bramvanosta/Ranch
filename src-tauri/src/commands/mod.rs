//! Command surface for Ranch: the operations (`*_inner`) and their Tauri
//! commands.
//!
//! Every command is an operation of the catalogue
//! (`application::catalogue`), which says which shells expose it; the
//! parity tests fail when a shell and the catalogue differ.

// Without the GUI shell the `#[tauri::command]` wrappers are compiled out:
// what only they used (imports, a few args structs) would otherwise warn.
#![cfg_attr(not(feature = "desktop"), allow(unused_imports, dead_code))]

pub mod ask_ai;
pub mod attention;
pub mod audit;
pub mod cause_notes;
pub mod connections;
pub mod groups;
pub mod hierarchy;
pub mod jobs;
pub mod mcp_activity;
pub mod mcp_installer;
pub mod mcp_safety;
pub mod queues;
pub mod safety;
pub mod settings;
pub mod updater;
pub mod windows;

#[cfg(test)]
mod unsupported_tests;

/// Single source of truth for the tauri-specta command registration.
///
/// Used both by `lib.rs::run` (to build the actual invoke handler at app
/// launch) AND by `tests/codegen.rs` (to regenerate `bindings.ts` headlessly
/// via `cargo test --test codegen`). Adding a new command means appending
/// it here AND to the catalogue (`application::catalogue`);
/// `tests/parity_contract.rs` fails when the two differ.
#[cfg(feature = "desktop")]
pub fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    use crate::commands::jobs::BulkProgressEvent;
    use crate::contract::audit::{
        AuditAction, AuditLogEntry, AuditOutcome, AuditSource, AuditTarget,
    };
    use crate::contract::connections::{BullmqRedisEndpoint, ReadOnlyState};
    use crate::contract::jobs::{Job, JobRef, JobState};
    use crate::contract::queues::{CronJob, Queue, QueueId, QueueMetrics, StateCounts};
    use crate::contract::schema::{BullmqSchema, SchemaError};

    tauri_specta::Builder::<tauri::Wry>::new()
        .commands(tauri_specta::collect_commands![
            // Connection wizard
            connections::parse_redis_url,
            connections::test_connection,
            connections::add_connection,
            connections::list_connections,
            connections::list_backends,
            // Sidebar
            connections::ping_connection,
            connections::rename_environment,
            // Direct env enumeration (MCP-driven, also exposed to UI for parity).
            connections::list_environments,
            settings::get_kv_setting,
            settings::set_kv_setting,
            // Manage connections
            connections::update_connection,
            connections::duplicate_connection,
            connections::remove_connection,
            // Color is an attribute of the environment.
            hierarchy::change_environment_color,
            // Environments as entities + moving connections.
            hierarchy::create_environment,
            hierarchy::delete_environment,
            hierarchy::move_connection,
            // MCP safety settings (UI-only, MCP-forbidden).
            mcp_safety::get_mcp_safety_settings,
            mcp_safety::set_mcp_safety_settings,
            mcp_safety::set_ai_features,
            mcp_safety::set_connection_mcp_readonly,
            // The approval queue, GUI side (UI-only, MCP-forbidden).
            mcp_safety::list_pending_approvals,
            mcp_safety::get_approval_record,
            mcp_safety::list_approval_history,
            cause_notes::annotate_cause,
            cause_notes::list_cause_notes,
            cause_notes::dismiss_cause_note,
            ask_ai::launch_claude_code,
            ask_ai::open_agent_app,
            ask_ai::share_for_agents,
            mcp_safety::decide_pending_approval,
            // MCP activity log, GUI side (UI-only).
            mcp_activity::list_mcp_activity,
            mcp_activity::list_mcp_activity_sessions,
            mcp_activity::export_mcp_activity,
            mcp_activity::clear_mcp_activity,
            hierarchy::reorder_groups,
            hierarchy::reorder_environments,
            // Groups as first-class entities (UI-only).
            groups::list_groups,
            groups::create_group,
            groups::rename_group,
            groups::delete_group,
            groups::set_group_collapsed,
            connections::set_connection_collapsed,
            windows::open_window,
            windows::open_settings_window,
            // Queue list with live counters
            queues::list_queues,
            // Paginated job list per (queue, state)
            jobs::list_jobs,
            // Job drill-down (single fetch + BullMQ logs)
            jobs::get_job,
            jobs::get_job_logs,
            // Failed jobs across the queues of a connection.
            jobs::list_failed_jobs,
            jobs::list_failure_causes,
            // Recent activity on the overview pages.
            jobs::list_recent_jobs,
            // Enqueue a job
            jobs::add_job,
            // CRON / repeatable jobs view
            queues::list_cron_jobs,
            // Workers attached to a queue (CLIENT LIST)
            queues::list_workers,
            queues::count_queue_workers,
            // Flows
            jobs::get_flow,
            queues::list_flow_roots,
            // Job schedulers
            queues::preview_schedule,
            queues::schedule_runs_within,
            queues::upsert_scheduler,
            queues::remove_scheduler,
            // Queue maintenance
            queues::preview_queue_maintenance,
            queues::drain_queue,
            queues::clean_queue,
            queues::retry_all_failed,
            // Audit log viewer + export
            audit::list_audit_log_entries,
            audit::export_audit_log,
            // per-connection read-only lock + unlock UX
            safety::get_connection_lock_state,
            safety::unlock_connection,
            safety::lock_connection,
            // single-job destructive actions
            jobs::retry_job,
            jobs::remove_job,
            jobs::promote_job,
            // Bulk single-queue destructive actions
            jobs::bulk_retry_jobs,
            jobs::bulk_remove_jobs,
            jobs::bulk_promote_jobs,
            // Pause / resume queue + CRON scheduler
            queues::pause_queue,
            queues::resume_queue,
            queues::pause_scheduler,
            queues::resume_scheduler,
            // 24h observability dashboards.
            queues::get_queue_metrics,
            queues::get_connection_metrics,
            queues::get_throughput,
            queues::summarize_window,
            queues::get_topology,
            queues::follow_connection_events,
            queues::get_server_info,
            queues::read_queue_events,
            attention::list_attention,
            // MCP installer commands.
            mcp_installer::get_mcp_installer_status,
            mcp_installer::install_mcp_for_claude_desktop,
            mcp_installer::install_mcp_for_cursor,
            // Generic installer (Claude Code, Codex, Windsurf, …).
            mcp_installer::install_mcp_for_client,
            mcp_installer::get_mcp_manual_snippet,
            // Auto-update orchestration.
            updater::check_for_updates,
            updater::install_update,
        ])
        // Register the contract types the front end names on their own so they
        // appear in `bindings.ts` even when no command references them
        // directly; most are also referenced transitively via command return
        // values, and the explicit `.typ::<T>()` keeps them all in scope.
        .typ::<QueueId>()
        .typ::<JobRef>()
        .typ::<JobState>()
        .typ::<Queue>()
        .typ::<StateCounts>()
        .typ::<QueueMetrics>()
        .typ::<Job>()
        .typ::<CronJob>()
        .typ::<BullmqSchema>()
        .typ::<SchemaError>()
        // Audit log domain types
        .typ::<AuditAction>()
        .typ::<AuditTarget>()
        .typ::<AuditSource>()
        .typ::<AuditOutcome>()
        .typ::<AuditLogEntry>()
        // read-only lock state
        .typ::<ReadOnlyState>()
        // The endpoint of a `bullmq-redis` connection: a connection carries
        // it as JSON, the connection form reads and writes it as this type.
        .typ::<BullmqRedisEndpoint>()
        // bulk:progress event payload. Not a command
        // return value, so it has to be explicitly registered to
        // appear in `bindings.ts` for the frontend event listener.
        .typ::<BulkProgressEvent>()
        // The queue:events event payload.
        .typ::<crate::commands::queues::QueueEventsNotice>()
}
