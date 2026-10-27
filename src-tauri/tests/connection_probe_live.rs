//! Live test for the new-connection sheet's probe.
//!
//!   docker compose --profile redis --profile bullmq up -d
//!   RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test connection_probe_live

use std::sync::Arc;

use ranch_lib::commands::connections::{test_connection_inner_with_keychain, TestConnectionArgs};
use ranch_lib::core::keychain::{InMemoryKeychain, KeychainAdapter};

#[tokio::test]
async fn a_successful_test_says_what_bullmq_holds_there() {
    if std::env::var("RANCH_LOCAL_REDIS").is_err() {
        eprintln!("skipping: set RANCH_LOCAL_REDIS=1 (see the module doc)");
        return;
    }
    let port: u16 = std::env::var("RANCH_REDIS_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(6390);
    let keychain: Arc<dyn KeychainAdapter> = Arc::new(InMemoryKeychain::new());
    let result = test_connection_inner_with_keychain(
        TestConnectionArgs {
            host: "127.0.0.1".into(),
            port,
            db: 0,
            username: None,
            password: None,
            tls: false,
            tls_custom_ca: None,
            ssh_tunnel: None,
            ssh_password: None,
            saved_connection_id: None,
        },
        keychain,
    )
    .await
    .expect("test");
    assert!(result.success, "{:?}", result.error);
    let probe = result.probe.expect("probe");
    // The seed: emails, images, reports (bull prefix); emails has 2 schedulers.
    assert!(probe.queues_found >= 3, "{probe:?}");
    assert!(
        probe.queue_sample.iter().any(|q| q == "bull:emails"),
        "{probe:?}"
    );
    assert!(probe.schedulers >= 2, "{probe:?}");
    assert!(probe.workers.is_some(), "a local Redis lists its clients");
}
