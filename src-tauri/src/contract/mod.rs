//! The operations' arguments and results, as the desktop app, the HTTP
//! server and the MCP server exchange them. `bindings.ts` is generated
//! from these types. A type that carries domain or core values mirrors
//! the type of the same name and converts from it; an argument converts
//! into the query a port or a core function takes.

pub mod audit;
pub mod capabilities;
pub mod cause_notes;
pub mod connections;
pub mod errors;
pub mod hierarchy;
pub mod jobs;
pub mod mcp;
pub mod queues;
pub mod schema;
