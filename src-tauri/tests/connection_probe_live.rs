//! Live test for the new-connection sheet's probe.
//!
//!   docker compose --profile redis --profile bullmq up -d
//!   RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 cargo test --test connection_probe_live -- --include-ignored

use std::sync::Arc;

use ranch_lib::adapters::secrets::InMemoryKeychain;
use ranch_lib::application::ports::SecretStore;
use ranch_lib::commands::connections::{test_connection_inner_with_keychain, TestConnectionArgs};

#[tokio::test]
#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]
async fn a_successful_test_says_what_bullmq_holds_there() {
    let port: u16 = std::env::var("RANCH_REDIS_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(6390);
    let keychain: Arc<dyn SecretStore> = Arc::new(InMemoryKeychain::new());
    let result = test_connection_inner_with_keychain(
        TestConnectionArgs {
            backend: "bullmq-redis".into(),
            endpoint: ranch_lib::commands::connections::BullmqRedisEndpoint {
                host: "127.0.0.1".into(),
                port,
                db: 0,
                username: None,
                tls: false,
                tls_custom_ca: None,
                ssh_tunnel: None,
            }
            .json(),
            password: None,
            ssh_password: None,
            saved_connection_id: None,
        },
        &ranch_lib::adapters::bullmq_redis::specs(),
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
