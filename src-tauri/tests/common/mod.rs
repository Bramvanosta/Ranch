//! Helpers shared by integration tests. Each test file that needs them
//! declares `mod common;`.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A slow link: a local TCP proxy to Redis that adds `one_way`
/// in each direction (round trip += 2 × `one_way`). Returns its port.
pub async fn slow_proxy(redis_port: u16, one_way: Duration) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let server = TcpStream::connect(("127.0.0.1", redis_port)).await.unwrap();
                let (cr, cw) = client.into_split();
                let (sr, sw) = server.into_split();
                tokio::spawn(delayed_copy(cr, sw, one_way));
                tokio::spawn(delayed_copy(sr, cw, one_way));
            });
        }
    });
    port
}

async fn delayed_copy(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    delay: Duration,
) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(tokio::time::Instant, Vec<u8>)>();
    tokio::spawn(async move {
        while let Some((due, bytes)) = rx.recv().await {
            tokio::time::sleep_until(due).await;
            if to.write_all(&bytes).await.is_err() {
                return;
            }
        }
    });
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match from.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                let _ = tx.send((tokio::time::Instant::now() + delay, buf[..n].to_vec()));
            }
        }
    }
}
