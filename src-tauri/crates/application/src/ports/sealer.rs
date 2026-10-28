//! Seals on the state that decides what an agent may do.
//!
//! `ranch-mcp` and the app share `ranch.db`. Any process of the user can
//! write that file (an agent with a shell included), so on its own the
//! database cannot prove that a human approved a call, or that a connection
//! is open to agent writes. The app therefore seals, with a key only it and
//! `ranch-mcp` hold:
//!
//!   - each approval it grants (`mcp_pending_approvals.decision_seal`);
//!   - the safety settings ([`SafetySetting`](super::settings::SafetySetting),
//!     a `seal:<key>` row each, written by the settings store alone);
//!   - each connection's agent read-only flag and lock
//!     (`connections.safety_seal`, written by the connection repository
//!     alone, in the transaction that changes them).
//!
//! `ranch-mcp` checks the seals and treats a missing or wrong one as the
//! most restrictive value: no write, approval required, AI off, payloads
//! withheld. The app seals whenever it writes one of these values; the
//! first run that creates the key seals the state as it finds it.

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

/// A connection's agent read-only flag and lock.
pub fn connection_parts(id: &str, mcp_readonly: Option<i64>, is_locked: i64) -> [&str; 4] {
    ["connection", id, flag(mcp_readonly), flag(Some(is_locked))]
}

/// An approval: what the human saw and agreed to.
pub fn approval_parts<'a>(
    id: &'a str,
    connection_id: &'a str,
    tool: &'a str,
    arguments_json: &'a str,
) -> [&'a str; 5] {
    ["approval", id, connection_id, tool, arguments_json]
}
