# 0004. `ranch-mcp` trusts seals, not the database

## Context

`ranch-mcp` and the app share `ranch.db`. Any process of the user can write that file, an agent with a shell included, so a row saying "approved" or "AI on" could have been written by the agent it is meant to restrain.

## Decision

The app seals, with an HMAC keyed by a secret in the OS keychain (`ranch::integrity`), each approval it grants and every value that governs agents: the AI switch, the approval mode, the agent read-only default and per-connection flags, the lock, where a connection points, the job-data sharing mode. `ranch-mcp` checks each seal and reads a missing or wrong one as the strictest value; it opens no connection whose seal does not hold, since the row says where its password is sent. Approval state changes are conditional updates whose affected-row count decides the outcome, so an approval is consumed once.

## Consequences

- Tampering with the database can only make `ranch-mcp` stricter.
- Every write path to a sealed value must seal it; the seal tests (`tests/mcp_integrity.rs`) cover them.
- On Linux the Secret Service hands the key to any process of the user, as it does the Redis passwords: the guarantee is as strong as the keychain.
- Debug builds started on a throwaway database (`RANCH_MCP_DATA_DIR`) skip seal checks, for tests.
