//! Entry points that speak a client's protocol: each converts its input,
//! calls the operations, and shapes the answer for its client. The
//! desktop commands are in `commands`, the HTTP server in `server`.

pub mod mcp;
