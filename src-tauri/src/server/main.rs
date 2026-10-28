//! `ranch-server`: Ranch without a window.
//!
//! Serves the built frontend and `POST /api/invoke/{command}` on the same
//! Rust core as the desktop app. Configuration is environment variables only
//! (see `ranch_lib::server::config`). Built without the default features:
//!
//!   cargo build --release --no-default-features --features server --bin ranch-server

use ranch_lib::server::{run, ServerConfig};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // `ranch-server --healthcheck`: the image's HEALTHCHECK. The runtime image
    // has no shell and no curl; the binary asks itself.
    if std::env::args().any(|a| a == "--healthcheck") {
        std::process::exit(if healthy().await { 0 } else { 1 });
    }

    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .try_init();

    let config = match ServerConfig::from_env() {
        Ok(config) => config,
        Err(reason) => {
            eprintln!("ranch-server: {reason}");
            std::process::exit(2);
        }
    };
    if let Err(e) = run(config).await {
        eprintln!("ranch-server: {e}");
        std::process::exit(1);
    }
}

/// `GET /healthz` on the local port, by hand: no HTTP client in the binary
/// for one request.
async fn healthy() -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let port = std::env::var("RANCH_PORT").unwrap_or_else(|_| "8080".to_string());
    let attempt = async {
        let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await?;
        stream
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await?;
        let mut head = [0u8; 12];
        stream.read_exact(&mut head).await?;
        Ok::<bool, std::io::Error>(&head == b"HTTP/1.1 200")
    };
    matches!(
        tokio::time::timeout(std::time::Duration::from_secs(3), attempt).await,
        Ok(Ok(true))
    )
}
