//! Who asks for a write. The audit row names them, and an agent may
//! carry the human approval that lets one operation through a lock.

use super::AuditSource;

#[derive(Debug, Clone)]
pub enum Caller {
    /// The person at the Ranch window (or the Ranch server's user).
    User,
    /// An agent over MCP.
    Agent {
        /// The MCP client's name (`claude-desktop`, …).
        client: String,
        /// How it reached Ranch: `stdio`.
        transport: String,
        /// Set when a human approved this call in Ranch. Only
        /// `core::mcp::gate` mints a grant.
        approval: Option<ApprovalGrant>,
    },
}

impl Caller {
    pub fn agent(client: impl Into<String>, transport: impl Into<String>) -> Self {
        Self::Agent {
            client: client.into(),
            transport: transport.into(),
            approval: None,
        }
    }

    /// Attach the approval the MCP gate obtained. A user needs none.
    pub fn with_approval(self, grant: Option<ApprovalGrant>) -> Self {
        match self {
            Self::Agent {
                client, transport, ..
            } => Self::Agent {
                client,
                transport,
                approval: grant,
            },
            Self::User => Self::User,
        }
    }

    pub fn source(&self) -> AuditSource {
        match self {
            Self::User => AuditSource::Ui,
            Self::Agent { .. } => AuditSource::Agent,
        }
    }

    pub fn agent_client(&self) -> Option<&str> {
        match self {
            Self::User => None,
            Self::Agent { client, .. } => Some(client),
        }
    }

    pub fn agent_transport(&self) -> Option<&str> {
        match self {
            Self::User => None,
            Self::Agent { transport, .. } => Some(transport),
        }
    }

    pub fn approval(&self) -> Option<&ApprovalGrant> {
        match self {
            Self::User => None,
            Self::Agent { approval, .. } => approval.as_ref(),
        }
    }
}

/// Proof that a human approved one specific tool call. The connection actor
/// trades it (once) for the right to execute even on a session-locked
/// connection: the approval *is* the explicit human gesture, scoped to a
/// single operation.
///
/// The fields are private. The MCP gate (`core::mcp::gate`) mints a grant
/// once the app's seal over the approved call checks out; nothing else in
/// Ranch calls [`ApprovalGrant::new`]. A grant is worthless without its
/// `approved` row: the write pipeline trades the id against the approval
/// store (`ApprovalStore::consume`), once, for this connection and tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalGrant {
    approval_id: String,
    tool: String,
}

impl ApprovalGrant {
    /// The MCP gate's, once a human approved the call.
    pub fn new(approval_id: String, tool: String) -> Self {
        Self { approval_id, tool }
    }

    pub fn approval_id(&self) -> &str {
        &self.approval_id
    }

    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// Test seam for the approval-grant matrix (forged / stale ids). Production code
    /// has no path to this outside `core::mcp`.
    #[doc(hidden)]
    pub fn forge_for_test(approval_id: &str, tool: &str) -> Self {
        Self::new(approval_id.to_string(), tool.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_is_audited_as_the_ui_and_carries_no_approval() {
        let user =
            Caller::User.with_approval(Some(ApprovalGrant::forge_for_test("a1", "retry_job")));
        assert_eq!(user.source(), AuditSource::Ui);
        assert_eq!((user.agent_client(), user.agent_transport()), (None, None));
        assert!(user.approval().is_none());
    }

    #[test]
    fn an_agent_names_its_client_and_keeps_its_approval() {
        let agent = Caller::agent("claude-desktop", "stdio")
            .with_approval(Some(ApprovalGrant::forge_for_test("a1", "retry_job")));
        assert_eq!(agent.source(), AuditSource::Agent);
        assert_eq!(agent.agent_client(), Some("claude-desktop"));
        assert_eq!(agent.agent_transport(), Some("stdio"));
        assert_eq!(agent.approval().map(|g| g.approval_id()), Some("a1"));
    }
}
