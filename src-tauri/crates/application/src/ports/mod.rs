//! The ports the application needs. Each is a trait with an adapter in
//! `adapters` and, where tests need one, an in-memory double.

mod approvals;
mod audit;
pub mod backend;
pub mod clock;
pub mod connections;
pub mod progress;
pub mod sealer;
pub mod secrets;
pub mod settings;

pub use approvals::ApprovalStore;
pub use audit::AuditLog;
pub use backend::{require, BackendSpec, BackendSpecs, Backends, QueueBackend};
pub use clock::Clock;
pub use connections::ConnectionRepository;
pub use progress::ProgressSink;
pub use ranch_domain::capabilities::{ensure, Capabilities};
pub use sealer::Sealer;
pub use secrets::SecretStore;
pub use settings::SettingsStore;
