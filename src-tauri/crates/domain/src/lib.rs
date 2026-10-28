//! Ranch's types and rules, apart from the I/O that feeds them: queues,
//! jobs, flows, workers, schedulers, the sidebar hierarchy, what a
//! connection points at (its backend and endpoint) and what a backend
//! declares it can do; the lock rule and the production defaults, the
//! names and colours of the hierarchy, failures grouped by cause, schedule
//! validation and next runs, and the metrics arithmetic; what the audit log
//! records, the SSH tunnel a connection's endpoint may name, and the error
//! every operation answers. Nothing here talks to Redis, SQLite or the app,
//! and nothing derives a wire format: `contract` mirrors what crosses a
//! boundary. Each rule is unit-tested on its own.
//!
//! A crate of its own so that the dependency rule holds by construction:
//! its manifest names no store, shell or wire format, and no layer above it.

// `RanchError`'s variants carry the identity fields the UI renders, which
// puts it past clippy's `result_large_err` threshold; boxing them would
// lose that (the app crate allows it for the same reason).
#![allow(clippy::result_large_err)]

pub mod audit;

pub mod capabilities;
pub mod endpoint;
pub mod errors;
pub mod events;
pub mod failures;
pub mod flows;
pub mod hierarchy;
pub mod jobs;
pub mod lock;
pub mod maintenance;
pub mod metrics;
pub mod probe;
pub mod production;
pub mod queues;
pub mod schedule;
pub mod server;
pub mod ssh_config;
pub mod timeline;
pub mod topology;
pub mod workers;
