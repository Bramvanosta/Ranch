# 0007. Seals are made inside the stores

## Context

[0004](0004-ranch-mcp-trusts-seals-not-the-database.md) has the app seal every value that governs agents and `ranch-mcp` check each seal. As first written, sealing was a step each caller had to take: a command that wrote a safety setting or a connection's lock called a reseal function after its write, and each reader called a "protected" read and interpreted the verdict. A new write path that forgot the reseal would leave a value `ranch-mcp` reads as tampered with; a new read path that used the plain getter would trust a value any process could have written. The key itself sat in a process-wide slot, set once at startup, which every caller reached implicitly.

## Decision

The stores seal, not their callers. `SettingsStore` seals a safety setting (the AI switch, the approval mode, the agent read-only default, the job-data sharing mode and patterns) on every write and removal of it, refuses to write a `seal:` row, and reads it only through `safety`, which returns the value together with the seal's verdict. `ConnectionRepository` seals a connection's agent read-only flag and lock in the same transaction as any write that changes them, and reports whether that seal holds when the gate asks. Approvals are sealed when decided and checked when polled, through the same port.

The key lives in a `Sealer` instance, built by each composition root and handed to the stores and the approval queue: `HmacSealer::for_app` in the desktop app, `for_agents` in `ranch-mcp`, `unsealed` in Ranch server and on a debug build's throwaway database. What a missing or wrong seal means is unchanged: `ranch-mcp` reads it as the strictest value.

## Consequences

- An operation cannot write a safety setting or a connection's flags without sealing them, and cannot read a safety setting without being told whether its seal holds.
- Tests choose their sealer: they seal nothing unless they build an enforcing one, as `tests/mcp_integrity.rs` does, and two tests no longer share a process-wide key.
- The app still reads the plain value of a safety setting through `get` (the settings screen does); only the `safety` read carries a verdict, and only `ranch-mcp` checks it.
- Another store for these values must seal inside itself too, and pass the seal tests.
- A connection's seal covers its id, its agent read-only flag, its lock, and its backend and endpoint as stored: every write of where a connection points goes through the repository, which seals it again, and a change of how an endpoint is stored needs its seals carried over.
