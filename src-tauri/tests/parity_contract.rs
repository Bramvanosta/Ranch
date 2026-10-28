//! The UI/MCP parity contract, as the catalogue of operations
//! (`application::catalogue`) states it.
//!
//! **The contract is asymmetric: `UI ⊇ MCP`.** Every operation is a UI
//! command, and an MCP tool is one way to reach an operation, so every
//! tool has its UI command by construction. UI-only commands (form
//! helpers, lock toggles, admin operations) stay UI-only by design.
//!
//! What the data cannot say alone is checked by the live suites, which
//! read the same catalogue: every MCP write tool through the write
//! pipeline refuses a locked connection (`lock_sentinel_live.rs`) and
//! leaves an audit row (`mcp_writes_live.rs`).

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use ranch_lib::application::catalogue::{self, Kind, Trail, OPERATIONS};
use tauri_specta::{BuilderConfiguration, LanguageExt};

#[test]
fn the_counts_are_pinned() {
    // The UI count is every command Ranch registers. The MCP count is the
    // agent-facing subset: the read tools, plus the write tools that go
    // through the safety gate. The rest is UI-only on purpose
    // (installers, settings, approval and lock controls, windows, form
    // helpers) or MCP-forbidden (anything that would let an agent loosen
    // its own limits or read its own approval trail).
    assert_eq!(OPERATIONS.len(), 97, "expected 97 UI commands");
    assert_eq!(catalogue::tools().count(), 29, "expected 29 MCP tools");
}

/// Explicitly enumerate the MCP surface so any addition / removal forces a
/// conscious update of this test.
#[test]
fn mcp_surface_is_exactly_the_declared_tools() {
    let mcp: HashSet<&str> = catalogue::tools().map(|(name, _)| name).collect();
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

/// The tools whose name is not their command's.
#[test]
fn aliases_are_pinned() {
    let aliases: Vec<(&str, &str)> = catalogue::tools()
        .filter(|(name, op)| *name != op.name)
        .map(|(name, op)| (name, op.name))
        .collect();
    assert_eq!(
        aliases,
        [
            ("inspect_job", "get_job"),
            ("list_schedulers", "list_cron_jobs")
        ]
    );
}

/// Every MCP write tool goes through the write pipeline (the lock, the
/// audit log) unless the catalogue says why not; the exceptions are
/// pinned so that a new one is a conscious change.
#[test]
fn only_documented_write_tools_skip_the_write_pipeline() {
    let outside: Vec<&str> = catalogue::tools()
        .filter(|(_, op)| matches!(op.kind, Kind::Write(Trail::Outside(_))))
        .map(|(name, _)| name)
        .collect();
    assert_eq!(outside, ["annotate_cause"]);
}

/// A write through the pipeline acts on a connection's backend, so it
/// names what it needs of it.
#[test]
fn every_queue_write_needs_a_capability() {
    for op in OPERATIONS {
        if op.kind == Kind::Write(Trail::Pipeline) {
            assert!(
                op.needs.is_some(),
                "{} writes a connection's queues without naming the capability it needs",
                op.name
            );
        }
    }
}

/// The Tauri commands are the catalogue's operations: `specta_builder`'s
/// `collect_commands!` registers each one, and nothing else.
#[test]
fn the_tauri_commands_are_the_catalogues_operations() {
    /// An "export" that keeps the names of the registered commands.
    struct CommandNames(Arc<Mutex<Vec<String>>>);
    impl LanguageExt for CommandNames {
        type Error = std::io::Error;
        fn export(self, cfg: &BuilderConfiguration, _: &Path) -> Result<(), Self::Error> {
            let names = cfg.commands.iter().map(|f| f.name().to_string());
            self.0.lock().unwrap().extend(names);
            Ok(())
        }
    }

    let names = Arc::new(Mutex::new(Vec::new()));
    ranch_lib::commands::specta_builder()
        .export(CommandNames(names.clone()), "")
        .unwrap();
    let mut registered = names.lock().unwrap().clone();
    registered.sort();
    let mut operations: Vec<String> = OPERATIONS.iter().map(|op| op.name.to_string()).collect();
    operations.sort();
    assert_eq!(
        registered, operations,
        "commands/mod.rs::specta_builder and the catalogue's operations differ"
    );
}
