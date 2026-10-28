//! What checking a connection finds: whether the server answered, how
//! fast, why not when it did not, and what it holds.

/// Why a server could not be reached or would not talk. A check that
/// fails this way is a result, not an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    AuthFailed,
    CannotReach {
        reason: String,
    },
    Timeout,
    TlsHandshake {
        reason: String,
    },
    /// The SSH tunnel failed before the server was reached.
    SshTunnel {
        reason: String,
    },
    Other {
        reason: String,
    },
}

/// A reachability check of a saved connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PingReport {
    pub success: bool,
    pub latency_ms: Option<u32>,
    pub error: Option<ProbeError>,
}

/// What an endpoint is checked with: the connection's secret and the SSH
/// tunnel's password, when it logs in with one. Never stored or returned;
/// `Debug` leaves them out.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ProbeSecrets {
    pub password: Option<String>,
    pub ssh_password: Option<String>,
}

impl std::fmt::Debug for ProbeSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProbeSecrets")
            .field("password", &self.password.as_ref().map(|_| "…"))
            .field("ssh_password", &self.ssh_password.as_ref().map(|_| "…"))
            .finish()
    }
}

/// The SSH host key a check went through, pinned on first use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyPin {
    pub fingerprint: String,
    /// The key was pinned by this check, rather than found pinned.
    pub was_new_pin: bool,
}

/// What a server that answered holds: how fast it answered, and its queues,
/// schedulers and workers, each found within a bound.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProbeFindings {
    /// Round trip of one ping, ms.
    pub latency_ms: u32,
    pub queues_found: u32,
    /// A few of the queues, sorted.
    pub queue_sample: Vec<String>,
    /// The search stopped before the end: there may be more queues.
    pub scan_capped: bool,
    pub schedulers: u32,
    /// `None` when the server will not say.
    pub workers: Option<u32>,
}

/// Checking an endpoint before it is saved: whether the server answered,
/// its version, why not, the SSH host key it went through, and what it
/// holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    pub success: bool,
    pub version: Option<String>,
    pub error: Option<ProbeError>,
    /// Set when the check went through an SSH tunnel it opened, whether
    /// or not the server then answered.
    pub host_key: Option<HostKeyPin>,
    pub findings: Option<ProbeFindings>,
}
