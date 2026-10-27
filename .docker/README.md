# Local services for Ranch development

Quick recipes for spinning up Redis (plain + TLS) and seeding BullMQ data
so the queue/job views have something to render.

Everything lives behind `docker compose` profiles — `docker compose up` (no
profile) starts NOTHING. Pick what you need.

## Plain Redis on `:6390`

For the `RANCH_LOCAL_REDIS=1` scaling soak and the live counter
polling tests. **Note**: host port is `6390`, NOT `6379`, to avoid
colliding with other dev Redis instances you might have running.

```bash
docker compose --profile redis up -d
```

Verify:

```bash
docker exec ranch-redis redis-cli ping        # PONG
```

Scaling test (gated):

```bash
RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 \
  cd src-tauri && \
  cargo test --test connection_actor_scaling -- --ignored --nocapture
```

## TLS Redis on `:6391`

For a manual TLS smoke test. Host port is `6391` (container's TLS
port is `6380`). **First-time setup** generates the test CA + server cert
(30-day validity, regenerable):

```bash
./scripts/gen-test-tls-certs.sh
docker compose --profile tls up -d
```

In Ranch:
1. Add a new connection.
2. URL: `rediss://localhost:6391` → toggle TLS auto-fills.
3. Open the **Advanced TLS** section.
4. Paste the contents of `.docker/tls/ca.crt` into "Custom CA certificate".
5. Click **Test connection** → should return Redis version.

Without the custom CA, the test fails with a verbatim rustls handshake
error (verifies the broadened `TLS_NEEDLES` heuristic).

## BullMQ-seeded queues

For the queue and job views. Loads three queues (`emails`, `images`,
`reports`) into the plain Redis service with a mix of waiting / delayed /
failed / completed jobs.

```bash
docker compose --profile redis --profile bullmq up
```

The `bullmq-seed` container runs once and exits. Jobs persist in Redis
until you `docker compose down` or `redis-cli flushdb`.

## Stop everything

```bash
docker compose down
```

## Notes

- All ports bind to `127.0.0.1` only — never exposed beyond localhost.
- TLS certs (`.docker/tls/{ca,server}.{crt,key}`) are gitignored. Run
  `scripts/gen-test-tls-certs.sh` on each new dev machine.
- The `bullmq-seed` container's `node_modules/` is gitignored.
- These services are LOCAL DEVELOPMENT ONLY. Do not deploy them.
