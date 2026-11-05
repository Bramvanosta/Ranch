//! What the connection overview's info strip says about the
//! Redis server: version, memory, operations per second, clients. Pure:
//! the actor sends `INFO`, this reads the answer.

pub use ranch_domain::server::ServerInfo;

/// Read the fields the strip shows out of an `INFO` reply. Missing or
/// unreadable fields stay `None` (managed Redis services hide some).
pub fn parse_info(info: &str, latency_ms: u32) -> ServerInfo {
    let number = |name: &str| field(info, name).and_then(|v| v.parse::<f64>().ok());
    let (server_name, redis_version) = server_identity(info);
    ServerInfo {
        server_name,
        redis_version,
        latency_ms,
        used_memory_bytes: number("used_memory"),
        max_memory_bytes: number("maxmemory").filter(|v| *v > 0.0),
        ops_per_sec: number("instantaneous_ops_per_sec"),
        connected_clients: number("connected_clients"),
    }
}

/// Which server answers `INFO server`, and its own version. Valkey says
/// `redis_version:7.2.4` to the clients that check it, and names itself
/// in `server_name` with its version in `valkey_version`; Redis has no
/// `server_name`. `None` for the name means Redis.
pub fn server_identity(info: &str) -> (Option<String>, Option<String>) {
    let name = field(info, "server_name").filter(|n| !n.is_empty() && *n != "redis");
    let version = name
        .and_then(|n| field(info, &format!("{n}_version")))
        .or_else(|| field(info, "redis_version"))
        .map(str::to_string);
    let display = name.map(|n| {
        let mut c = n.chars();
        c.next()
            .map(|f| f.to_uppercase().chain(c).collect())
            .unwrap_or_default()
    });
    (display, version)
}

fn field<'a>(info: &'a str, name: &str) -> Option<&'a str> {
    info.lines()
        .find_map(|l| l.trim_end().strip_prefix(name)?.strip_prefix(':'))
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
                server_name: None,
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

    #[test]
    fn valkey_by_its_own_name_and_version() {
        let info =
            "# Server\r\nredis_version:7.2.4\r\nserver_name:valkey\r\nvalkey_version:9.1.2\r\n";
        let info = parse_info(info, 1);
        assert_eq!(info.server_name.as_deref(), Some("Valkey"));
        assert_eq!(info.redis_version.as_deref(), Some("9.1.2"));
        // A server that names itself without its own version: the Redis one.
        assert_eq!(
            server_identity("redis_version:7.2.4\nserver_name:other\n"),
            (Some("Other".into()), Some("7.2.4".into()))
        );
        // Redis, even when it fills `server_name` in.
        assert_eq!(
            server_identity("redis_version:8.0.0\nserver_name:redis\n"),
            (None, Some("8.0.0".into()))
        );
    }
}
