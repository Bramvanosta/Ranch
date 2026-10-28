//! The rule every write follows, in one place:
//!
//!   1. authorize: the caller passes the connection's lock, or trades the
//!      human approval it carries (once, for this connection and tool);
//!   2. the backend acts;
//!   3. the outcome goes to the audit log before the caller hears of it.
//!
//! The connection's write loop runs steps 1 to 3 for one write at a time,
//! so a lock change never lands in the middle of one.

use std::sync::Arc;

use ranch_domain::audit::{AuditAction, AuditEntry, AuditOutcome, AuditTarget, Caller};
use ranch_domain::errors::RanchError;
use ranch_domain::lock::{check_lock_or_err, ReadOnlyState};

use super::ports::{ApprovalStore, AuditLog};

/// The connection as the error and the audit row name it.
#[derive(Debug, Clone)]
pub struct ConnectionStamp {
    pub id: String,
    pub name: String,
    pub env_color: Option<String>,
}

pub struct WritePipeline {
    audit: Arc<dyn AuditLog>,
    approvals: Arc<dyn ApprovalStore>,
}

impl WritePipeline {
    pub fn new(audit: Arc<dyn AuditLog>, approvals: Arc<dyn ApprovalStore>) -> Self {
        Self { audit, approvals }
    }

    /// Step 1. Without an approval, the lock decides. With one, it must be
    /// consumed for this connection and this tool; then the write runs even
    /// on a locked connection, the approval being the human's explicit
    /// gesture for this one operation. Any other approval (forged, pending,
    /// denied, expired, used, for another connection or tool) refuses.
    pub async fn authorize(
        &self,
        connection: &ConnectionStamp,
        lock: ReadOnlyState,
        caller: &Caller,
        tool: &str,
    ) -> Result<(), RanchError> {
        let Some(grant) = caller.approval() else {
            return check_lock_or_err(lock, &connection.id, &connection.name);
        };
        let valid = grant.tool() == tool
            && self
                .approvals
                .consume(grant.approval_id(), &connection.id, tool)
                .await?;
        if valid {
            Ok(())
        } else {
            Err(RanchError::ApprovalInvalid {
                tool: tool.to_string(),
            })
        }
    }

    /// Step 3. A failure here fails the write's reply: the caller must not
    /// see as done what the log does not hold.
    pub async fn record(
        &self,
        connection: &ConnectionStamp,
        caller: &Caller,
        action: AuditAction,
        target: AuditTarget,
        outcome: AuditOutcome,
        reason: Option<String>,
    ) -> Result<(), RanchError> {
        self.audit
            .append(AuditEntry {
                source: caller.source(),
                action,
                target,
                outcome,
                outcome_reason: reason,
                connection_id: Some(connection.id.clone()),
                connection_name: Some(connection.name.clone()),
                env_color: connection.env_color.clone(),
                agent_client: caller.agent_client().map(str::to_string),
                agent_transport: caller.agent_transport().map(str::to_string),
                approval_id: caller.approval().map(|g| g.approval_id().to_string()),
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use ranch_domain::audit::{ApprovalGrant, AuditSource};

    #[derive(Default)]
    struct Log(Mutex<Vec<AuditEntry>>);

    #[async_trait]
    impl AuditLog for Log {
        async fn append(&self, entry: AuditEntry) -> Result<(), RanchError> {
            self.0.lock().unwrap().push(entry);
            Ok(())
        }
    }

    /// Approves one id for one connection and tool, once.
    struct OneApproval(Mutex<Option<(String, String, String)>>);

    #[async_trait]
    impl ApprovalStore for OneApproval {
        async fn consume(&self, id: &str, conn: &str, tool: &str) -> Result<bool, RanchError> {
            let mut slot = self.0.lock().unwrap();
            let matches = slot
                .as_ref()
                .is_some_and(|(i, c, t)| i == id && c == conn && t == tool);
            if matches {
                *slot = None;
            }
            Ok(matches)
        }
    }

    fn pipeline() -> (WritePipeline, Arc<Log>) {
        let log = Arc::new(Log::default());
        let approvals = Arc::new(OneApproval(Mutex::new(Some((
            "a1".into(),
            "conn".into(),
            "retry_job".into(),
        )))));
        (WritePipeline::new(log.clone(), approvals), log)
    }

    fn conn() -> ConnectionStamp {
        ConnectionStamp {
            id: "conn".into(),
            name: "prod".into(),
            env_color: Some("env-production".into()),
        }
    }

    fn approved(id: &str, tool: &str) -> Caller {
        Caller::agent("claude", "stdio")
            .with_approval(Some(ApprovalGrant::forge_for_test(id, tool)))
    }

    #[tokio::test]
    async fn the_lock_decides_without_an_approval() {
        let (p, _) = pipeline();
        assert!(p
            .authorize(&conn(), ReadOnlyState::Unlocked, &Caller::User, "retry_job")
            .await
            .is_ok());
        let err = p
            .authorize(&conn(), ReadOnlyState::Locked, &Caller::User, "retry_job")
            .await;
        assert!(matches!(err, Err(RanchError::ConnectionLocked { .. })));
    }

    #[tokio::test]
    async fn an_approval_passes_a_lock_once_for_its_own_tool() {
        let (p, _) = pipeline();
        let wrong_tool = p
            .authorize(
                &conn(),
                ReadOnlyState::Locked,
                &approved("a1", "remove_job"),
                "retry_job",
            )
            .await;
        assert!(matches!(
            wrong_tool,
            Err(RanchError::ApprovalInvalid { .. })
        ));
        assert!(p
            .authorize(
                &conn(),
                ReadOnlyState::Locked,
                &approved("a1", "retry_job"),
                "retry_job"
            )
            .await
            .is_ok());
        let reused = p
            .authorize(
                &conn(),
                ReadOnlyState::Locked,
                &approved("a1", "retry_job"),
                "retry_job",
            )
            .await;
        assert!(matches!(reused, Err(RanchError::ApprovalInvalid { .. })));
    }

    #[tokio::test]
    async fn an_unknown_approval_refuses_even_an_unlocked_connection() {
        let (p, _) = pipeline();
        let forged = p
            .authorize(
                &conn(),
                ReadOnlyState::Unlocked,
                &approved("forged", "retry_job"),
                "retry_job",
            )
            .await;
        assert!(matches!(forged, Err(RanchError::ApprovalInvalid { .. })));
    }

    #[tokio::test]
    async fn the_audit_row_names_the_caller_and_the_connection() {
        let (p, log) = pipeline();
        let target = AuditTarget::Queue {
            queue_id: None,
            queue_name: "emails".into(),
        };
        p.record(
            &conn(),
            &approved("a1", "pause_queue"),
            AuditAction::PauseQueue,
            target.clone(),
            AuditOutcome::Success,
            None,
        )
        .await
        .unwrap();
        let rows = log.0.lock().unwrap();
        let row = &rows[0];
        assert_eq!(row.source, AuditSource::Agent);
        assert_eq!(row.agent_client.as_deref(), Some("claude"));
        assert_eq!(row.approval_id.as_deref(), Some("a1"));
        assert_eq!(row.connection_name.as_deref(), Some("prod"));
        assert_eq!(row.env_color.as_deref(), Some("env-production"));
        assert_eq!(row.target, target);
    }
}
