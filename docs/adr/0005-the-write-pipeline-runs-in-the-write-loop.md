# 0005. The write pipeline runs in the connection's write loop

## Context

A write is authorized against the connection's lock, then performed, then audited. The lock lives in the connection's actor, in memory: an unlock lasts for the session, and lock changes are serialised with writes. Checking the lock outside the actor and writing inside it would let a lock land between the two.

## Decision

The rule is one application service, `WritePipeline` (`crates/application/src/writes.rs`), behind ports for the audit log and the approval store. The actor owns the order: it calls `authorize` with its live lock state, performs the write, then calls `record`, all in one turn of its write loop. Shells and operations never authorize or audit a write themselves.

## Consequences

- The rule is written once and tested with in-memory ports, without Redis or SQLite.
- The actor needs no database handle: it holds the pipeline.
- Another backend gets the same guarantees by running the same three calls around its own serialised writes.
