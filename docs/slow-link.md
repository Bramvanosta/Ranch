# Ranch on a slow link

How to see and test Ranch against a Redis that is far away (a remote environment, an SSH tunnel, a VPN) without one.

## The bench

Ranch's own Redis, a delay in front of it, the server build serving the real UI, and BullMQ busy enough that things move.

1. **Redis with data**: Ranch's compose, never another project's Redis:

   ```bash
   docker compose --profile redis --profile bullmq up -d
   ```

2. **Activity**: a job every 250 ms across six queues, processed with some failures (the seed installed `bullmq`):

   ```bash
   node .docker/bullmq-seed/load.mjs
   ```

3. **The slow link**: 50 ms each way, so 100 ms round trip, on port 6490; `--control` lets you change it while it runs:

   ```bash
   echo 50 > /tmp/ranch-delay
   python3 -I scripts/latency-proxy.py 6490 6390 50 --control /tmp/ranch-delay
   ```

4. **Ranch**: the web build served by `ranch-server` (a debug build is enough), then add the connection `redis://127.0.0.1:6490/0` in the browser:

   ```bash
   pnpm build
   cd src-tauri && cargo build --bin ranch-server --no-default-features --features server
   RANCH_ALLOW_NO_AUTH=1 RANCH_DATA_DIR=/tmp/ranch-bench RANCH_STATIC_DIR=$PWD/../dist \
     RANCH_BIND=127.0.0.1 RANCH_PORT=8787 target/debug/ranch-server
   ```

   The server runtime does not follow BullMQ's event streams (only the desktop app does): it shows the polling path. `pnpm tauri dev` against the same proxy shows the live one.

5. **Clean up**: stop the three processes, then `docker compose down`.

## What to look at

- **Request timings** in the browser: every command is `POST /api/invoke/<name>`. A `PerformanceObserver` on `resource` entries gives their durations.
- **What Ranch asks Redis**: `docker exec ranch-redis redis-cli MONITOR` (Ranch's connections send `CLIENT SETINFO … redis-rs` when they open; a persistent connection opens once).
- **Connections opened**: `docker exec ranch-redis redis-cli INFO stats | grep total_connections_received`, twice, some seconds apart.

At 100 ms round trip (6 queues): queue list ~105 ms, a job page ~0.3 s, a job ~105 ms, the connection overview's metrics ~0.4 s; one Redis connection for the whole app, no SCAN between registry refreshes. At 600 ms everything still works.

## Budgets the tests hold

The live suites ([testing.md](testing.md#against-redis-and-real-bullmq-workers)) put the same delay in front of Redis with an in-test proxy (`src-tauri/tests/common/mod.rs`):

| Test | Budget |
|---|---|
| `persistent_connection_live` | ten commands open **one** Redis connection; a 600 ms link pings in ~600 ms instead of "cannot reach" |
| `queue_registry_live` | with the registry warm, every queue's counters in **under 2 round trips** |
| `concurrent_reads_live` | a ping is answered while a 5 s read is still going |
| `pipelined_reads_live` | a job page under 2.5 round trips; recent jobs of every queue under 3.5; the Workers tab's calls share one `CLIENT LIST` |
| `event_follow_live` | events written right after following are reported, with their queue |
