# Testing

What the tests cover, where they live and how to run them. Rust paths are relative to `src-tauri/`; [architecture.md](architecture.md) explains the layers they test.

## Everyday commands

```bash
pnpm check                       # Biome format and lint, types, Vitest, the scripts' tests
(cd src-tauri && cargo test --workspace)
pnpm check:overflow              # Playwright, when you touched the UI
```

A plain `cargo test --workspace` needs no Redis: the suites that do are skipped (see [below](#against-redis-and-real-bullmq-workers)).

## The Rust tests, from the inside out

| Layer | What | Where |
|---|---|---|
| Domain | The rules that need no I/O: the lock, production defaults, failures grouped by cause, schedules, metrics, the hierarchy's names. | unit tests beside the code in `crates/domain/src/` |
| Application | The write pipeline with in-memory ports; the catalogue's own invariants. | `crates/application/src/` |
| Operations | The `*_inner` functions on the in-memory backend (`adapters/fake_backend/`) and in-memory stores: the lock, approvals, the audit rows, refusing what a backend does not declare. | `tests/operations_on_fake_backend.rs`, `tests/capability_descriptor.rs`, `commands/unsupported_tests.rs` |
| Backend contract | One suite every backend passes: listing and counters, job actions, adding jobs, pausing, maintenance, schedulers, workers, the lock and the audit rows. | `tests/common/backend_contract.rs`, run by `tests/backend_contract.rs` (in memory) and `tests/backend_contract_live.rs` (BullMQ) |
| BullMQ on Redis | The connection actor and every read and write against a real Redis; the schema decoders against real job hashes of BullMQ 5 and 6. | `tests/*_live.rs`; `tests/bullmq_schema_versions.rs` with `crates/bullmq-redis/src/bullmq_schema/fixtures/` |
| BullMQ compatibility | Real BullMQ 5.x and 6.x workers (`.docker/bullmq-compat/`) process what Ranch's Lua scripts wrote: added, retried, scheduled, cleaned jobs and flows. | `tests/*_compat_live.rs` |
| Persistence | The migrations (a fresh database, and one in an earlier shape upgraded with its rows converted), the audit log's durability (reopened after each write), seals, settings, connections and their keychain entries. | `tests/db_migrations.rs`, `tests/audit_durability.rs`, `tests/mcp_integrity.rs`, `tests/settings_kv.rs`, `tests/connections_*.rs`, unit tests in `crates/sqlite/` and `crates/secrets/` |
| MCP | The tools called directly, the approval flow, the activity log, job-data sharing, and the real `ranch-mcp` binary over stdio against a throwaway database (`RANCH_MCP_DATA_DIR`, debug builds only). | `tests/mcp_smoke.rs`, `tests/mcp_approval.rs`, `tests/mcp_activity.rs`, `tests/share_live.rs`, `tests/mcp_ai_disabled.rs` |
| Ranch server | Every command served over HTTP or refused as desktop-only, auth and OIDC. | `server/tests.rs`, `server/tests_oidc.rs` |

Integration tests use a temporary SQLite database and `InMemoryKeychain`. `tests/common/` holds the shared helpers: the contract suite, the BullMQ seed, the Redis target, the stdio client for `ranch-mcp`, and a slow TCP proxy for the latency budgets in [slow-link.md](slow-link.md).

### The shells, held to the catalogue

The catalogue (`crates/application/src/catalogue.rs`) is the one list of operations; each shell keeps the literal list its macros need, and a test fails when the two differ:

- `tests/parity_contract.rs`: the Tauri commands are the catalogue's operations; it pins the command count, the exact MCP tool set, the aliases and the write tools outside the write pipeline, so any addition is deliberate.
- `server/tests.rs` (`the_routes_are_the_catalogues_served_commands`): the HTTP routes are the commands the catalogue serves.
- `tests/mcp_tool_schemas.rs`: `tools/list`, with every capability declared, is the catalogue's tools; annotations agree with each operation's kind; the server's instructions list the tools served. It also pins what an agent of the BullMQ kind sees (names, descriptions, input schemas) in `tests/snapshots/mcp_tools.json`. After an intended change, regenerate and commit it:

  ```bash
  cd src-tauri && RANCH_UPDATE_SNAPSHOTS=1 cargo test --test mcp_tool_schemas
  ```

- `tests/mcp_safety.rs`: every exposed tool is in the catalogue, the write set is the expected one, every write is refused on a connection read-only for agents, and no forbidden operation is exposed. `tests/lock_sentinel_live.rs` and `tests/mcp_writes_live.rs` call every write tool through the real `ranch-mcp`: refused on a locked connection before Redis, and otherwise changing Redis and leaving an audit row signed by the agent. A new write tool fails them until they know how to call it.
- `tests/dependency_rules.rs`: the crates' manifests list no forbidden dependency, and `core/` and `adapters/` derive no wire format.
- `tests/codegen.rs` regenerates `src/lib/tauri/bindings.ts`; commit the result with the change that caused it.

The `server` feature has its own run, as CI's server job does:

```bash
cd src-tauri && cargo test --workspace --no-default-features --features server --lib
```

### Against Redis and real BullMQ workers

A test that needs Redis is `#[ignore = "needs Redis: RANCH_LOCAL_REDIS=1 (CI runs it)"]`: a plain `cargo test` skips it and says why. Run them against Ranch's own Redis (port 6390, never another project's):

```bash
docker compose --profile redis --profile bullmq up -d       # Redis on :6390, seeded queues
(cd .docker/bullmq-compat && npm ci)                         # BullMQ 5 and 6 workers
cd src-tauri
RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 RANCH_BULLMQ_COMPAT=1 \
  cargo test --workspace --test '*_live' -- --include-ignored
```

- `RANCH_LOCAL_REDIS=1` turns the live suites on; `RANCH_REDIS_HOST` and `RANCH_REDIS_PORT` (default `127.0.0.1:6390`) choose the Redis.
- `RANCH_BULLMQ_COMPAT=1` makes a missing Node harness a failure instead of a skip in the `*_compat_live` suites.
- `RANCH_SSH_LIVE=1` runs the SSH tunnel tests in `tests/ssh_tunnel_runtime.rs` against a bastion; its module doc says how to start one.

CI's `desktop` job runs the same on every pull request, against a `redis:7-alpine` service on port 6379 seeded by `.docker/bullmq-seed/`, with `RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6379 RANCH_BULLMQ_COMPAT=1`.

## The front end

Vitest with Testing Library on jsdom (`vitest.config.ts`), tests beside the code as `src/**/*.test.ts(x)`; `pnpm test` also runs `scripts/*.test.mjs` with `node --test`.

- **Helpers** in `src/test/`: `factories.ts` (`aJob`, `aQueueId`… with every field filled in, so a test states only what it is about), `renderWithProviders.tsx` (a fresh query client per test that already holds the BullMQ kind's capabilities; `testQueryClient(backends)` for another kind), `results.ts` (`ok(data)` and `err(kind)` for mocked commands), `pressChord.ts` (a registry shortcut as the platform sends it).
- **The fake backend** (`src/backend/fake/`), the demo world of Ranch web, typed against the bindings; `fake.test.ts` drives its handlers directly.
- **Guards** that hold a rule review alone would miss: `src/features/boundaries.test.ts` (features meet through their `index.ts`, shared layers import no feature, no circle between features, only `src/data/` calls the backend commands) and `src/capabilities/neutralStates.test.ts` (production code names none of BullMQ's own states outside an allow-list with reasons). `src/styles/tokens.test.ts` checks the colour tokens' contrast and `src/styles/noClipping.test.ts` refuses the classes that would cut text (`truncate`, `line-clamp`…).

## Playwright

`pnpm check:overflow` runs every project in `playwright.config.ts` against Ranch web, starting `pnpm dev` on port 1420 or reusing one. Run `pnpm exec playwright install chromium` once first.

- **Overflow** (`e2e/overflow/`): every screen in `routes.ts`, dark and light, at 1440×900 and 1100×700. It fails on text cut off or ellipsised, running into its neighbour, or leaving the window, printing the screen, theme, size and text; the full results go to `e2e/.overflow-report.json`. `e2e/overflow-allowlist.json` is empty and stays so: fix a failure instead of listing it, and a stale entry fails the run.
- **Contrast** (`e2e/contrast/`): text under WCAG AA on any screen, in either theme.
- **Sheets** (`e2e/sheets/`): the sheets and the palette, for overflow and contrast.
- **AI off** (`e2e/ai/`): with AI features off, no screen shows the AI colour or the ✦ glyph.

A new screen adds its route to `e2e/overflow/routes.ts`. CI runs the check in the `mcr.microsoft.com/playwright:v<version>-noble` image of the `@playwright/test` version pinned in `package.json`, so bumping the package bumps the image.

## What CI runs

`.github/workflows/ci.yml`, on every pull request and on `main`:

| Job | Runs |
|---|---|
| `frontend` | `scripts/check-no-creds-in-sql.sh`, `pnpm lint`, `pnpm typecheck`, `pnpm test` |
| `overflow` | `pnpm check:overflow` in Playwright's image |
| `server` | `cargo fmt --check`; Clippy and `cargo test --lib` on the workspace with `--no-default-features --features server` |
| `desktop` | Clippy with `--workspace --all-targets -D warnings`, `cargo test --workspace`, then the live suites against Redis and BullMQ workers |
| `image` | pull requests: the server image, built and smoke-tested (`scripts/smoke-test-image.sh`) |
