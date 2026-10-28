//! What the connection overview's info strip says about the
//! Redis server: version, memory, operations per second, clients. Pure:
//! the actor sends `INFO`, this reads the answer.

pub use ranch_domain::server::ServerInfo;

/// Read the fields the strip shows out of an `INFO` reply. Missing or
/// unreadable fields stay `None` (managed Redis services hide some).
pub fn parse_info(info: &str, latency_ms: u32) -> ServerInfo {
    let field = |name: &str| -> Option<&str> {
        info.lines()
            .find_map(|l| l.trim_end().strip_prefix(name)?.strip_prefix(':'))
    };
    let number = |name: &str| field(name).and_then(|v| v.parse::<f64>().ok());
    ServerInfo {
        redis_version: field("redis_version").map(str::to_string),
        latency_ms,
        used_memory_bytes: number("used_memory"),
        max_memory_bytes: number("maxmemory").filter(|v| *v > 0.0),
        ops_per_sec: number("instantaneous_ops_per_sec"),
        connected_clients: number("connected_clients"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = "# Server\r\nredis_version:7.2.4\r\nredis_mode:standalone\r\n\r\n# Clients\r\nconnected_clients:12\r\n\r\n# Memory\r\nused_memory:2154632\r\nused_memory_human:2.05M\r\nmaxmemory:0\r\n\r\n# Stats\r\ninstantaneous_ops_per_sec:37\r\n";

    #[test]
    fn reads_the_fields_the_strip_shows() {
        let info = parse_info(INFO, 3);
        assert_eq!(
            info,
            ServerInfo {
                redis_version: Some("7.2.4".into()),
                latency_ms: 3,
                used_memory_bytes: Some(2_154_632.0),
                max_memory_bytes: None,
                ops_per_sec: Some(37.0),
                connected_clients: Some(12.0),
            }
        );
        // `used_memory_human` does not shadow `used_memory`.
        assert_eq!(
            parse_info("used_memory_human:2M\nused_memory:5\n", 0).used_memory_bytes,
            Some(5.0)
        );
    }

    #[test]
    fn a_hidden_or_odd_field_is_none() {
        let info = parse_info("# Server\nredis_version:7.0.0\nmaxmemory:1048576\n", 1);
        assert_eq!(info.max_memory_bytes, Some(1_048_576.0));
        assert_eq!(info.ops_per_sec, None);
        assert_eq!(info.used_memory_bytes, None);
        assert_eq!(parse_info("", 1).redis_version, None);
    }
}
