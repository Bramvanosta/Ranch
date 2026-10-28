//! What Ranch does, independent of where its data lives: the catalogue of
//! its operations, the rules every operation follows and the ports they
//! reach the outside world through.
//! Nothing here names Redis, SQLite or Tauri; `adapters` implements the
//! ports.
//!
//! A crate of its own so that the dependency rule holds by construction:
//! its manifest names the domain and no store, shell or wire format, and no
//! layer above it.

// Its ports answer `RanchError`; see the domain crate's note on
// `result_large_err`.
#![allow(clippy::result_large_err)]

pub mod catalogue;
pub mod ports;
pub mod writes;

pub use writes::{ConnectionStamp, WritePipeline};
