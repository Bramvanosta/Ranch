//! `POST /api/invoke/{command}` → the same `*_inner` function the Tauri
//! command of that name calls.
//!
//! One route per command, written to mirror its `#[tauri::command]` wrapper:
//! same inner function, same side effects around it (an edited connection's
//! actor is dropped so it respawns with the new settings, …). The request
//! body is the object the frontend hands to Tauri's `invoke`: `{ "args": … }`
//! for most commands, named fields (`{ "connectionId": … }`) for a few.
//!
//! What differs from the desktop shell, by design:
//!   - no window → no live `bulk:progress` events (progress goes to
//!     `NoProgress`); the operation's result is the report;
//!   - secrets come from `FileSecretStore` (`adapters::secrets`);
//!   - what the catalogue marks `Server::Refused` answers `desktopOnly` with
//!     the catalogue's reason: the MCP client installers, for one, edit the
//!     config files of agent clients on *this* machine, which in a container
//!     is nobody's machine.
//!
//! The routes are one list, from which `routes!` emits both the `match` and
//! [`SERVED`]. `tests::the_routes_are_the_catalogues_served_commands`
//! compares [`SERVED`] with what the catalogue says the server serves, and
//! `tests::every_ui_command_is_served_or_desktop_only` calls every command:
//! a command added to the app and forgotten here fails the build, not a
//! user.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::adapters::progress::NoProgress;
use crate::app_state::AppState;
use crate::application::catalogue::{self, Server};
use crate::commands::{
    audit, connections, groups, hierarchy, jobs, mcp_activity, mcp_safety, queues, safety,
    settings, updater,
};
use crate::contract::errors::RanchErrorDto;
use crate::contract::mcp;
use crate::contract::queues::QueueMaintenance;
use crate::core::mcp::{activity, approval};
use crate::domain::audit::Caller;
use crate::domain::errors::RanchError;

pub const KIND_UNKNOWN_COMMAND: &str = "unknownCommand";
pub const KIND_DESKTOP_ONLY: &str = "desktopOnly";

pub async fn dispatch(
    state: &AppState,
    command: &str,
    body: Value,
) -> Result<Value, RanchErrorDto> {
    if let Some(Server::Refused(why)) = catalogue::operation(command).map(|op| op.server) {
        return Err(RanchErrorDto {
            kind: KIND_DESKTOP_ONLY.to_string(),
            message: why.to_string(),
            doc_url: None,
        });
    }
    route(state, command, body).await
}

/// Emits [`SERVED`] and `route`'s `match` from one list of routes, so the
/// commands the shell says it serves are the ones it matches.
macro_rules! routes {
    (
        $(#[$meta:meta])*
        async fn $route:ident($state:ident, $command:ident, $body:ident) {
            $(let $local:ident = $init:expr;)*
        }
        $($name:literal => $handler:expr,)*
    ) => {
        /// Every command a route serves, in the routes' order.
        pub const SERVED: &[&str] = &[$($name),*];

        $(#[$meta])*
        async fn $route(
            $state: &AppState,
            $command: &str,
            $body: Value,
        ) -> Result<Value, RanchErrorDto> {
            $(let $local = $init;)*
            match $command {
                $($name => $handler,)*
                _ => Err(RanchErrorDto {
                    kind: KIND_UNKNOWN_COMMAND.to_string(),
                    message: format!("Unknown command: {}", $command),
                    doc_url: None,
                }),
            }
        }
    };
}

routes! {
    /// The command's route; `unknownCommand` when there is none.
    async fn route(state, command, body) {
        let pool = state.db.pool();
        let repository = state.repository.as_ref();
        let settings_store = state.settings.as_ref();
        let keychain = &state.keychain;
        let registry = state.connections.as_ref();
        let backends = state.backends.as_ref();
    }
    // ── connections ─────────────────────────────────────────────────
    "parse_redis_url" => reply(connections::parse_redis_url_inner(field(&body, "url")?).await),
    "test_connection" => {
        // A saved password only goes back to the endpoint it was saved for.
        let a = connections::resolve_saved_secrets(
            args(&body)?,
            repository,
            backends.specs(),
            keychain.as_ref(),
        )
        .await
        .map_err(RanchErrorDto::from)?;
        reply(
            connections::test_connection_inner_with_keychain(
                a,
                backends.specs(),
                keychain.clone(),
            )
            .await,
        )
    },
    "add_connection" => {
        let a = args(&body)?;
        reply(
            connections::add_connection_inner(a, repository, keychain, backends.specs()).await,
        )
    },
    "list_connections" => reply(connections::list_connections_inner(repository).await),
    "list_backends" => reply(Ok::<_, RanchError>(connections::list_backends_inner(
        backends.specs(),
    ))),
    "ping_connection" => {
        let id: String = field(&body, "connectionId")?;
        reply(connections::ping_connection_inner(&id, backends).await)
    },
    "list_environments" => reply(connections::list_environments_inner(repository).await),
    "rename_environment" => {
        let a: connections::RenameEnvironmentArgs = args(&body)?;
        reply(
            connections::rename_environment_inner(&a.environment_id, &a.new_label, repository)
                .await,
        )
    },
    "update_connection" => {
        let a: connections::UpdateConnectionArgs = args(&body)?;
        reply(
            connections::update_connection_inner(
                a,
                repository,
                keychain,
                backends.specs(),
                Some(registry),
            )
            .await,
        )
    },
    "duplicate_connection" => {
        let a: connections::DuplicateConnectionArgs = args(&body)?;
        reply(connections::duplicate_connection_inner(&a.id, repository, keychain).await)
    },
    "remove_connection" => {
        let id: String = field(&body, "id")?;
        reply(
            connections::remove_connection_inner(&id, repository, keychain, Some(registry))
                .await,
        )
    },

    // ── groups & environments ───────────────────────────────────────
    "list_groups" => reply(groups::list_groups_inner(repository).await),
    "create_group" => {
        let a: groups::CreateGroupArgs = args(&body)?;
        reply(groups::create_group_inner(&a.name, repository).await)
    },
    "rename_group" => {
        let a: groups::RenameGroupArgs = args(&body)?;
        reply(groups::rename_group_inner(&a.id, &a.name, repository).await)
    },
    "set_connection_collapsed" => {
        let a: connections::SetConnectionCollapsedArgs = args(&body)?;
        reply(
            connections::set_connection_collapsed_inner(
                &a.connection_id,
                a.collapsed,
                repository,
            )
            .await,
        )
    },
    "set_group_collapsed" => {
        let a: groups::SetGroupCollapsedArgs = args(&body)?;
        reply(groups::set_group_collapsed_inner(&a.id, a.collapsed, repository).await)
    },
    "delete_group" => {
        let a: groups::DeleteGroupArgs = args(&body)?;
        reply(
            groups::delete_group_inner(&a.id, a.cascade, repository, keychain, Some(registry))
                .await,
        )
    },
    "reorder_groups" => {
        let a: hierarchy::ReorderGroupsArgs = args(&body)?;
        reply(hierarchy::reorder_groups_inner(&a.ordered_ids, repository).await)
    },
    "change_environment_color" => {
        let a: hierarchy::ChangeEnvironmentColorArgs = args(&body)?;
        let recolored = hierarchy::change_environment_color_inner(
            &a.environment_id,
            &a.color_token,
            repository,
        )
        .await;
        match recolored {
            Ok(ids) => {
                // The actor keeps the color and lock it was spawned with:
                // respawn the environment's connections so both apply now.
                for id in ids {
                    registry.shutdown(&id).await;
                }
                reply(Ok::<(), RanchError>(()))
            }
            Err(e) => Err(e.into()),
        }
    },
    "create_environment" => {
        let a: hierarchy::CreateEnvironmentArgs = args(&body)?;
        reply(
            hierarchy::create_environment_inner(
                &a.group_id,
                &a.name,
                &a.color_token,
                repository,
            )
            .await,
        )
    },
    "move_connection" => {
        let a: hierarchy::MoveConnectionArgs = args(&body)?;
        let result =
            hierarchy::move_connection_inner(&a.connection_id, &a.environment_id, repository)
                .await;
        // The actor keeps the color (for the audit log) and the lock it was
        // spawned with: respawn it in its new environment.
        if result.is_ok() {
            registry.shutdown(&a.connection_id).await;
        }
        reply(result)
    },

    // ── settings ────────────────────────────────────────────────────
    "get_kv_setting" => {
        let key: String = field(&body, "key")?;
        reply(settings::get_kv_setting_inner(settings_store, &key).await)
    },
    "set_kv_setting" => {
        let key: String = field(&body, "key")?;
        let value: String = field(&body, "value")?;
        // Signed-out sessions are the server's to keep: no page brings one back.
        if key == super::REVOKED_SESSIONS_KEY {
            return Err(RanchError::InvalidInput {
                reason: format!("\"{key}\" is not a setting the UI writes"),
            }
            .into());
        }
        reply(settings::set_kv_setting_inner(settings_store, &key, &value).await)
    },

    // ── queues ──────────────────────────────────────────────────────
    "list_queues" => {
        let a: queues::ListQueuesArgs = args(&body)?;
        reply(queues::list_queues_inner(&a.connection_id, backends).await)
    },
    "list_cron_jobs" => reply(queues::list_cron_jobs_inner(args(&body)?, backends).await),
    "count_queue_workers" => {
        reply(queues::count_queue_workers_inner(args(&body)?, backends).await)
    },
    "list_workers" => reply(queues::list_workers_inner(args(&body)?, backends).await),
    "list_flow_roots" => reply(queues::list_flow_roots_inner(args(&body)?, backends).await),
    "schedule_runs_within" => reply(queues::schedule_runs_within_inner(
        &args(&body)?,
        chrono::Utc::now(),
    )),
    "preview_schedule" => {
        let schedule = field(&body, "schedule")?;
        reply(queues::preview_schedule_inner(
            &schedule,
            chrono::Utc::now(),
        ))
    },
    "upsert_scheduler" => {
        reply(queues::upsert_scheduler_inner(args(&body)?, backends, Caller::User).await)
    },
    "remove_scheduler" => {
        reply(queues::remove_scheduler_inner(args(&body)?, backends, Caller::User).await)
    },
    "preview_queue_maintenance" => {
        reply(queues::preview_queue_maintenance_inner(args(&body)?, backends).await)
    },
    "drain_queue" => {
        let a: queues::DrainQueueArgs = args(&body)?;
        maintenance(
            state,
            queues::QueueMaintenanceArgs {
                connection_id: a.connection_id,
                queue_id: a.queue_id,
                operation: QueueMaintenance::Drain {
                    include_delayed: a.include_delayed,
                },
            },
        )
        .await
    },
    "clean_queue" => {
        let a: queues::CleanQueueArgs = args(&body)?;
        maintenance(
            state,
            queues::QueueMaintenanceArgs {
                connection_id: a.connection_id,
                queue_id: a.queue_id,
                operation: QueueMaintenance::Clean {
                    state: a.state,
                    older_than_ms: a.older_than_ms,
                },
            },
        )
        .await
    },
    "retry_all_failed" => {
        let a: queues::QueueActionArgs = args(&body)?;
        maintenance(
            state,
            queues::QueueMaintenanceArgs {
                connection_id: a.connection_id,
                queue_id: a.queue_id,
                operation: QueueMaintenance::RetryAllFailed,
            },
        )
        .await
    },
    "pause_queue" => {
        reply(queues::pause_queue_inner(args(&body)?, backends, Caller::User).await)
    },
    "resume_queue" => {
        reply(queues::resume_queue_inner(args(&body)?, backends, Caller::User).await)
    },
    "pause_scheduler" => {
        reply(queues::pause_scheduler_inner(args(&body)?, backends, Caller::User).await)
    },
    "resume_scheduler" => {
        reply(queues::resume_scheduler_inner(args(&body)?, backends, Caller::User).await)
    },
    "get_queue_metrics" => reply(queues::get_queue_metrics_inner(args(&body)?, backends).await),
    "list_attention" => reply(
        crate::commands::attention::list_attention_inner(
            args(&body)?,
            pool,
            repository,
            backends,
        )
        .await,
    ),
    "read_queue_events" => reply(queues::read_queue_events_inner(args(&body)?, backends).await),
    "get_server_info" => reply(queues::get_server_info_inner(args(&body)?, backends).await),
    "get_throughput" => reply(queues::get_throughput_inner(args(&body)?, backends).await),
    // No window to send events to: `false`, the web app
    // keeps polling.
    "follow_connection_events" => reply(
        queues::follow_connection_events_inner(args(&body)?, backends, NoProgress::sink())
            .await,
    ),
    "get_topology" => reply(queues::get_topology_inner(args(&body)?, backends).await),
    "summarize_window" => reply(queues::summarize_window_inner(args(&body)?, backends).await),
    "get_connection_metrics" => {
        reply(queues::get_connection_metrics_inner(args(&body)?, backends).await)
    },

    // ── jobs ────────────────────────────────────────────────────────
    "list_jobs" => reply(jobs::list_jobs_inner(args(&body)?, backends).await),
    "get_job" => reply(jobs::get_job_inner(args(&body)?, backends).await),
    "get_job_logs" => reply(jobs::get_job_logs_inner(args(&body)?, backends).await),
    "get_flow" => reply(jobs::get_flow_inner(args(&body)?, backends).await),
    "list_failed_jobs" => reply(jobs::list_failed_jobs_inner(args(&body)?, backends).await),
    "list_failure_causes" => {
        reply(jobs::list_failure_causes_inner(args(&body)?, backends).await)
    },
    "list_recent_jobs" => reply(jobs::list_recent_jobs_inner(args(&body)?, backends).await),
    "add_job" => reply(jobs::add_job_inner(args(&body)?, backends, Caller::User).await),
    "retry_job" => reply(jobs::retry_job_inner(args(&body)?, backends, Caller::User).await),
    "remove_job" => reply(jobs::remove_job_inner(args(&body)?, backends, Caller::User).await),
    "promote_job" => reply(jobs::promote_job_inner(args(&body)?, backends, Caller::User).await),
    "bulk_retry_jobs" => reply(
        jobs::bulk_retry_jobs_inner(args(&body)?, backends, NoProgress::sink(), Caller::User)
            .await,
    ),
    "bulk_remove_jobs" => reply(
        jobs::bulk_remove_jobs_inner(args(&body)?, backends, NoProgress::sink(), Caller::User)
            .await,
    ),
    "bulk_promote_jobs" => reply(
        jobs::bulk_promote_jobs_inner(args(&body)?, backends, NoProgress::sink(), Caller::User)
            .await,
    ),

    // ── audit & safety ──────────────────────────────────────────────
    "list_audit_log_entries" => {
        reply(audit::list_audit_log_entries_inner(args(&body)?, pool).await)
    },
    "export_audit_log" => reply(audit::export_audit_log_inner(args(&body)?, pool).await),
    "get_connection_lock_state" => {
        reply(safety::get_connection_lock_state_inner(args(&body)?, backends).await)
    },
    "unlock_connection" => {
        reply(safety::unlock_connection_inner(args(&body)?, pool, backends).await)
    },
    "lock_connection" => {
        reply(safety::lock_connection_inner(args(&body)?, pool, backends).await)
    },

    // ── MCP, the part that is plain SQLite ──────────────────────────
    // The image runs no MCP server (out of scope there) and the web UI hides
    // these screens; the commands stay served so that the shared frontend
    // (which polls the approval queue) gets an empty list, not an error.
    "get_mcp_safety_settings" => {
        reply(mcp_safety::get_mcp_safety_settings_inner(settings_store).await)
    },
    "set_mcp_safety_settings" => {
        let s = field(&body, "settings")?;
        reply(mcp_safety::set_mcp_safety_settings_inner(&s, settings_store).await)
    },
    "set_connection_mcp_readonly" => {
        let a: mcp_safety::SetConnectionMcpReadonlyArgs = args(&body)?;
        reply(
            mcp_safety::set_connection_mcp_readonly_inner(
                &a.connection_id,
                a.mcp_readonly,
                repository,
            )
            .await,
        )
    },
    "list_pending_approvals" => reply(approval::list_pending(pool, settings_store).await.map(
        |pending| {
            pending
                .into_iter()
                .map(mcp::PendingApproval::from)
                .collect::<Vec<_>>()
        },
    )),
    "get_approval_record" => {
        let id: String = field(&body, "approvalId")?;
        reply(
            approval::record(pool, &id)
                .await
                .map(|r| r.map(mcp::ApprovalRecord::from)),
        )
    },
    "annotate_cause" => {
        let a: crate::contract::cause_notes::AnnotateCauseArgs = args(&body)?;
        reply(
            crate::core::cause_notes::annotate(pool, a.into(), None)
                .await
                .map(crate::contract::cause_notes::CauseNote::from),
        )
    },
    "list_cause_notes" => {
        let a: crate::contract::cause_notes::ListCauseNotesArgs = args(&body)?;
        reply(
            crate::core::cause_notes::list(pool, a.into())
                .await
                .map(|notes| {
                    notes
                        .into_iter()
                        .map(crate::contract::cause_notes::CauseNote::from)
                        .collect::<Vec<_>>()
                }),
        )
    },
    "dismiss_cause_note" => {
        let id: String = field(&body, "noteId")?;
        reply(crate::core::cause_notes::dismiss(pool, &id).await)
    },
    "list_approval_history" => {
        let limit: u32 = field(&body, "limit")?;
        reply(approval::history(pool, limit).await.map(|items| {
            items
                .into_iter()
                .map(mcp::ApprovalHistoryItem::from)
                .collect::<Vec<_>>()
        }))
    },
    "decide_pending_approval" => {
        let a: mcp_safety::DecidePendingApprovalArgs = args(&body)?;
        reply(
            approval::decide(
                pool,
                state.sealer.as_ref(),
                &a.approval_id,
                a.decision.into(),
                a.reason.as_deref(),
            )
            .await
            .map(mcp::DecideOutcome::from),
        )
    },
    "list_mcp_activity" => {
        let a: mcp_activity::ListMcpActivityArgs = args(&body)?;
        let filter = a.filter.into();
        let events = activity::list(pool, &filter, a.limit, a.offset).await;
        let total = activity::count(pool, &filter).await;
        reply(events.and_then(|events| {
            total.map(|total| mcp_activity::McpActivityPage {
                events: events.into_iter().map(Into::into).collect(),
                total,
            })
        }))
    },
    "list_mcp_activity_sessions" => {
        reply(activity::list_sessions(pool, 200).await.map(|sessions| {
            sessions
                .into_iter()
                .map(mcp::ActivitySession::from)
                .collect::<Vec<_>>()
        }))
    },
    "export_mcp_activity" => {
        let a: mcp_activity::ExportMcpActivityArgs = args(&body)?;
        reply(
            activity::export(
                pool,
                &a.filter.into(),
                a.format == mcp_activity::ActivityExportFormat::Csv,
            )
            .await,
        )
    },
    "clear_mcp_activity" => reply(
        activity::clear(pool)
            .await
            .map(|n| n.min(u32::MAX as u64) as u32),
    ),

    // ── updates ─────────────────────────────────────────────────────
    // An image is updated by pulling a tag, not by the app.
    "check_for_updates" => reply(Ok::<_, RanchError>(updater::UpdateCheckResult {
        available: false,
        version: None,
        notes: None,
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        disabled: true,
        disabled_reason: Some("Server image: update by pulling a newer tag".to_string()),
    })),

}

async fn maintenance(
    state: &AppState,
    args: queues::QueueMaintenanceArgs,
) -> Result<Value, RanchErrorDto> {
    reply(
        queues::run_queue_maintenance_inner(
            args,
            state.backends.as_ref(),
            NoProgress::sink(),
            Caller::User,
        )
        .await,
    )
}

/// `{ "args": … }`: the shape of almost every command.
fn args<T: DeserializeOwned>(body: &Value) -> Result<T, RanchErrorDto> {
    field(body, "args")
}

/// One named field of the request body. A missing field deserializes from
/// `null`, so an `Option` parameter may be left out and anything else says
/// what is missing.
fn field<T: DeserializeOwned>(body: &Value, key: &str) -> Result<T, RanchErrorDto> {
    let value = body.get(key).cloned().unwrap_or(Value::Null);
    serde_json::from_value(value).map_err(|e| {
        RanchError::InvalidInput {
            reason: format!("`{key}`: {e}"),
        }
        .into()
    })
}

fn reply<T: Serialize, E: Into<RanchErrorDto>>(
    result: Result<T, E>,
) -> Result<Value, RanchErrorDto> {
    let value = result.map_err(Into::into)?;
    serde_json::to_value(value).map_err(|e| {
        RanchError::InvalidInput {
            reason: format!("the result could not be encoded: {e}"),
        }
        .into()
    })
}
