//! The outside world behind the application's ports.

/// Queue backends for BullMQ on Redis: the `ranch-bullmq-redis` crate.
pub use ranch_bullmq_redis as bullmq_redis;
pub mod fake_backend;
pub mod hmac_sealer;
pub mod progress;
/// The secret stores: the `ranch-secrets` crate.
pub use ranch_secrets as secrets;
/// The stores kept in `ranch.db`: the `ranch-sqlite` crate.
pub use ranch_sqlite as sqlite;
pub mod system_clock;
