//! Integration test for the `SecretStore` trait contract.
//!
//! Uses `InMemoryKeychain` so this test runs cleanly on Linux CI runners
//! that don't have `secret-service`. The real `OsKeychain` is exercised
//! manually on macOS and indirectly via the commands that touch real
//! connections.

use ranch_lib::adapters::secrets::InMemoryKeychain;

#[test]
fn set_then_get_returns_stored_secret() {
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());
    kc.set("conn-1", "hunter2").expect("set");
    let got = kc.get("conn-1").expect("get");
    assert_eq!(got, Some("hunter2".to_string()));
}

#[test]
fn get_unknown_returns_none_not_error() {
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());
    let got = kc.get("does-not-exist").expect("get");
    assert_eq!(got, None);
}

#[test]
fn delete_then_get_returns_none() {
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());
    kc.set("conn-1", "hunter2").expect("set");
    kc.delete("conn-1").expect("delete");
    let got = kc.get("conn-1").expect("get after delete");
    assert_eq!(got, None);
}

#[test]
fn delete_unknown_is_noop() {
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());
    // Deleting a non-existent entry returns Ok(()), not Err.
    kc.delete("does-not-exist").expect("noop delete");
}

#[test]
fn overwriting_existing_secret_replaces_it() {
    let kc: std::sync::Arc<dyn ranch_lib::application::ports::SecretStore> =
        std::sync::Arc::new(InMemoryKeychain::new());
    kc.set("conn-1", "old").expect("set old");
    kc.set("conn-1", "new").expect("set new");
    let got = kc.get("conn-1").expect("get");
    assert_eq!(got, Some("new".to_string()));
}
