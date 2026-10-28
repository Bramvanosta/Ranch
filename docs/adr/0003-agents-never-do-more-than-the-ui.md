# 0003. Agents never do more than the UI

## Context

Ranch gives agents the user's queues over MCP. An agent that can do something the UI cannot is an agent whose actions the user cannot reproduce, inspect or undo through the interface they know.

## Decision

Every MCP tool maps to a UI command and calls the same operation; there is no agent-only path to Redis. The catalogue of operations (`crates/application/src/catalogue.rs`) lists every command and classifies every operation as read or write; `tests/parity_contract.rs` fails when a tool has no UI counterpart and pins the counts, so adding either is deliberate. Agent writes pass more checks than the user's (agent read-only, approval), never fewer.

## Consequences

- One implementation per operation: the lock, the audit log and the validations apply the same way to both.
- Some UI commands are not tools (connection management, unlocking, the updater): an agent cannot widen its own permissions.
- A new agent capability starts as a UI feature.
