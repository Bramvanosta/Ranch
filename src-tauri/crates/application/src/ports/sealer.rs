//! Seals on the state that decides what an agent may do.
//!
//! `ranch-mcp` and the app share `ranch.db`. Any process of the user can
//! write that file (an agent with a shell included), so on its own the
//! database cannot prove that a human approved a call, or that a connection
//! is open to agent writes. The app therefore seals, with a key only it and
//! `ranch-mcp` hold:
//!
//!   - each approval it grants, over the call and how it was shown
//!     (`mcp_pending_approvals.decision_seal`);
//!   - the safety settings ([`SafetySetting`](super::settings::SafetySetting),
//!     a `seal:<key>` row each, written by the settings store alone);
//!   - each connection's agent read-only flag and lock, and where it
//!     points: its backend and endpoint (`connections.safety_seal`, written
//!     by the connection repository alone, in the transaction that changes
//!     them).
//!
//! `ranch-mcp` checks the seals and treats a missing or wrong one as the
//! most restrictive value: no write, approval required, AI off, payloads
//! withheld. A connection whose seal does not hold is not opened at all,
//! reads included: its password would go wherever the row points. The app
//! seals whenever it writes one of these values; the first run that creates
//! the key seals the state as it finds it.

/// What a seal check says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// Sealed by the app, or seals are not checked in this process.
    Valid,
    /// Missing or wrong: someone else wrote the value.
    Invalid,
    /// Checked here, but the key could not be read from the keychain.
    Unverifiable,
}

/// Makes and checks seals. One per process, chosen by its composition
/// root: the app seals and checks nothing (it trusts its own writes),
/// `ranch-mcp` checks everything, a process without a key seals nothing.
pub trait Sealer: Send + Sync {
    /// The seal of `parts`, or `None` when this process has no key.
    fn seal(&self, parts: &[&str]) -> Option<String>;
    /// Whether `seal` is the seal of `parts`.
    fn check(&self, parts: &[&str], seal: Option<&str>) -> Check;
}

// ── what is sealed ──────────────────────────────────────────────────────────

/// A safety setting's value; an absent row is sealed too, so deleting it
/// does not bring back a default.
pub fn setting_parts<'a>(setting: &'a str, value: Option<&'a str>) -> [&'a str; 3] {
    ["setting", setting, value.unwrap_or("\u{0}absent")]
}

fn flag(value: Option<i64>) -> &'static str {
    match value {
        None => "null",
        Some(0) => "0",
        Some(_) => "1",
    }
}

/// A connection's agent read-only flag and lock, and its `backend` and
/// `endpoint` columns as stored: the server its password is sent to.
pub fn connection_parts<'a>(
    id: &'a str,
    mcp_readonly: Option<i64>,
    is_locked: i64,
    backend: &'a str,
    endpoint: &'a str,
) -> [&'a str; 6] {
    [
        "connection",
        id,
        flag(mcp_readonly),
        flag(Some(is_locked)),
        backend,
        endpoint,
    ]
}

/// What a connection's seal covered before it covered the endpoint. Only
/// read to carry those seals over (`ConnectionRepository::upgrade_seals`).
pub fn legacy_connection_parts(id: &str, mcp_readonly: Option<i64>, is_locked: i64) -> [&str; 4] {
    ["connection", id, flag(mcp_readonly), flag(Some(is_locked))]
}

/// What the approval sheet shows of a request's target: the connection as
/// it was displayed when the request was written, and the preview.
#[derive(Debug, Clone, Copy)]
pub struct ApprovalShown<'a> {
    pub connection_name: &'a str,
    pub group_name: &'a str,
    pub environment_name: &'a str,
    pub env_color: &'a str,
    pub target_preview_json: Option<&'a str>,
}

/// An approval: what the human saw and agreed to. The call itself, and how
/// it was shown: a request whose sheet was rewritten shows one thing and
/// runs nothing.
pub fn approval_parts<'a>(
    id: &'a str,
    connection_id: &'a str,
    tool: &'a str,
    arguments_json: &'a str,
    shown: ApprovalShown<'a>,
) -> [&'a str; 10] {
    [
        "approval",
        id,
        connection_id,
        tool,
        arguments_json,
        shown.connection_name,
        shown.group_name,
        shown.environment_name,
        shown.env_color,
        shown.target_preview_json.unwrap_or("\u{0}absent"),
    ]
}
