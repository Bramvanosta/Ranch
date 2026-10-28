# 0002. Writes are ports of BullMQ's own Lua scripts

## Context

BullMQ keeps a job's state in several keys (a hash, a list or sorted set per state, a marker, an event stream) and its workers rely on them being consistent. A client that moves a job with a handful of separate commands can leave a job in two states, or in none, if it is interrupted or races a worker.

## Decision

Every write that moves or creates jobs (retry, promote, add, drain, clean, retry all failed, scheduler upsert and removal) is a Lua script in `crates/bullmq-redis/src/bullmq_schema/lua/`, ported from the BullMQ script that does the same thing, so it runs atomically and leaves the keys as BullMQ would. Shared helpers live in `_prelude.lua`, ports of the BullMQ includes of the same name. Long operations run in batches, so Redis is never blocked for long and progress and cancellation happen between batches.

## Consequences

- What Ranch writes is processed by real BullMQ workers: the `*_compat_live` suites run BullMQ 5 and 6 on it.
- A new BullMQ major can change a script: the fixtures, the decoders and the compatibility suites are where that shows.
- The scripts are Ranch's to maintain; their header says where they come from.
