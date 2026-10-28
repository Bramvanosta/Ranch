//! The Redis server behind a connection, as the overview's info strip
//! shows it.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServerInfo {
    pub redis_version: Option<String>,
    /// Round trip of one `PING`, ms.
    pub latency_ms: u32,
    pub used_memory_bytes: Option<f64>,
    /// `maxmemory`; `None` when unlimited (0) or not reported.
    pub max_memory_bytes: Option<f64>,
    pub ops_per_sec: Option<f64>,
    pub connected_clients: Option<f64>,
}
