# 0001. One actor per connection

## Context

Ranch talks to Redis servers that are often remote: across an SSH tunnel, over TLS, on a slow link. A handshake per request makes every screen slow, and several screens read the same connection at once. Writes must not interleave: a retry and a drain on the same queue would race.

## Decision

Each connection in use gets one Tokio task (`crates/bullmq-redis/src/connection_actor/`) that owns its persistent Redis link, its SSH tunnel, the detected BullMQ schema and the lock state. Callers send it messages through a `ConnectionHandle`. Reads run concurrently on copies of its state (16 at a time); writes and lock changes run in the actor loop, one at a time. A blocking stream read gets its own link so it never holds the others.

## Consequences

- No handshake after the first request; a dropped link reconnects by itself.
- Writes on one connection are serialised without a lock in the callers.
- Every operation crosses a mailbox, so adding one touches the command enum, the handle and the registry.
- Editing a connection drops its actor, which respawns with the new settings and ends any session unlock.
