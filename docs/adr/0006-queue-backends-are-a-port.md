# 0006. Queue backends are a port

## Context

Ranch reads and changes queues that BullMQ keeps in Redis, but the rules around that work (the lock, approvals, the audit log, what agents may do, what the UI shows) do not depend on BullMQ or Redis. Operations that called the Redis engine directly would tie those rules to it, and another queue system would mean touching every operation.

## Decision

Operations reach queues only through a port in `crates/application/src/ports/`: `Backends` opens a connection's `QueueBackend` by id, resolving its settings and secrets itself. A `QueueBackend` hands out families of methods: reading queues and jobs always, writes, maintenance, queue control, schedules, workers, flows, metrics, events and server reads optionally. It also declares `Capabilities`, plain data with one flag per variant and the job states it keeps. An operation checks the flag it needs before calling and answers `Unsupported` naming the capability when it is not declared. Arguments and results are the domain's types; the operations convert them to and from the contract's.

BullMQ on Redis is one adapter (`crates/bullmq-redis/`, `adapters::bullmq_redis`), with the connection actor as its engine. An in-memory adapter (`adapters/fake_backend/`) keeps the same rules for tests. One suite, `tests/common/backend_contract.rs`, runs on both.

## Consequences

- Another backend is an adapter and a passing contract suite; operations, shells and the UI do not move. See [adding-a-backend.md](../adding-a-backend.md).
- Every backend must run writes through the write pipeline itself, one at a time per connection, as [0005](0005-the-write-pipeline-runs-in-the-write-loop.md) asks of the actor.
- Operations and their tests run on the in-memory backend, without Redis.
- The port speaks BullMQ's vocabulary (job states, schedulers, flows): a backend maps its own concepts onto it and declares what it cannot map.
- A kind of backend declares its capabilities on its spec, so they are known without connecting and sent to the front end as data. What a screen hides is a convenience: a backend that declares less is still refused, by name, when an operation reaches it.
