# Architecture

This page is a map of the code for contributors. It explains how the pieces fit together and where to look when you want to change something. For the user-facing features, see the [README](../README.md); for the agent tools, see [mcp-tools.md](mcp-tools.md); for the visual system, see [design.md](design.md).

## Contents

- [The big picture](#the-big-picture)
- [Three runtimes, one front end](#three-runtimes-one-front-end)
- [The Rust core](#the-rust-core)
- [Commands and the UI/MCP parity contract](#commands-and-the-uimcp-parity-contract)
- [The MCP server and agent approvals](#the-mcp-server-and-agent-approvals)
- [The front end](#the-front-end)
- [Testing](#testing)
- [Where to start for common changes](#where-to-start-for-common-changes)

## The big picture

Ranch is one Rust crate (`src-tauri/`) and one React app (`src/`). The crate builds three binaries from the same core:

| Binary | Cargo feature | What it is |
|---|---|---|
| `ranch` | `desktop` (default) | The Tauri desktop app. |
| `ranch-mcp` | `desktop` | The MCP server that agent clients start over stdio. |
| `ranch-server` | `server` | Ranch server: the same UI over HTTP (the Docker image). |

```text
                 React app (src/)
                        │  commands.xxx(...)  ← src/lib/tauri/bindings.ts
        ┌───────────────┼──────────────────────┐
        │               │                      │
   Tauri invoke    POST /api/invoke/{cmd}   in-memory world
   (desktop)       (Ranch server)           (Ranch web, dev only)
        │               │
        ▼               ▼
   #[tauri::command]   server/dispatch.rs
        └──────┬────────┘
               ▼
      commands/*.rs  `*_inner` functions  ◄──── core/mcp/server.rs (ranch-mcp)
               │
               ▼
      core/connection_actor  ── one actor per connection ──► Redis
               │                (SSH tunnel, TLS)
               ▼
      SQLite (ranch.db): connections, audit log, settings, approvals
```

Every operation is written once, as an `*_inner` function in `src-tauri/src/commands/`. The Tauri command, the HTTP route and the MCP tool are thin wrappers around it.

## Three runtimes, one front end

The same JavaScript bundle runs in three places. `src/main.tsx` decides which one at boot and records it with `setRuntime()` (`src/lib/runtime/runtime.ts`).

### Desktop (`desktop`)

The Tauri v2 app. Commands go through Tauri's `invoke`, events come from the Rust side (`queue:events`, `bulk:progress`, `mcp:pending_approval`), and the app has a native menu bar, native windows and tabs, a tray icon, deep links (`ranch://`), notifications and a self-updater. The entry point is `run()` in `src-tauri/src/lib.rs`, which registers the plugins, opens the database, builds `AppState` (`src-tauri/src/app_state.rs`) and starts the background tasks that agents need (the heartbeat and the approval watcher).

### Ranch server (`server`)

`ranch-server` (`src-tauri/src/server/`) is an Axum server that serves the built front end and one API route, `POST /api/invoke/{command}`. `server/dispatch.rs` maps each command name to the same `*_inner` function the Tauri command calls. It is built without Tauri, WebKit or the OS keychain:

```bash
cd src-tauri
cargo build --no-default-features --features server --bin ranch-server
```

What changes in this shell:

- **Auth.** HTTP Basic or OpenID Connect (`server/oidc.rs`, `server/session.rs`). The API also requires an `x-ranch-request` header, which a foreign page cannot make a browser send.
- **Secrets.** `server/secrets.rs` stores Redis and SSH passwords in a file in the data directory, encrypted with `RANCH_SECRET_KEY` when it is set. It implements the same `KeychainAdapter` trait as the OS keychain.
- **No window.** There are no live events. Bulk actions report when they finish, and the front end keeps polling instead of following event streams.
- **Desktop-only commands** (MCP setup, updater, native windows, Ask AI launchers) are listed in `DESKTOP_ONLY` and answer with a `desktopOnly` error.

On the front end, `src/lib/runtime/httpBackend.ts` turns `invoke(command, args)` into the HTTP call. `src/lib/runtime/tauriShim.ts` installs a stand-in for `window.__TAURI_INTERNALS__` so the Tauri JS plugins load and answer inertly.

To develop against a local server, run it and start Vite with `RANCH_SERVER_URL=http://127.0.0.1:8080 VITE_RANCH_BACKEND=http pnpm dev`. Vite proxies `/api` to it.

### Ranch web (`preview`)

`pnpm dev` opened in a plain browser loads `src/dev/webPreview.ts`: a stateful, in-memory BullMQ world behind the same shim. It holds groups (the demo group is "Lumen Studios"), a local / staging / locked production trio, an unreachable Redis, queues with jobs in every state, schedulers, workers, a flow, audit entries, agent activity and a pending approval. Writes change the in-memory world, and a reload starts over.

- `?preview=empty` shows a first launch; `?preview=full` brings the demo world back.
- `window.ranchPreview.requestApproval()` queues an agent approval; `reset()` rebuilds the world.

The preview is a dynamic `import()` behind `import.meta.env.DEV`, so it never ships in a production bundle. It is also what the Playwright checks run against.

### Asking what the runtime can do

Components ask capability questions, not "where am I": `hasAgents()` is false on the server, `hasUpdater()` only holds on the desktop. A native-only API must never be called when `getRuntime() === "server"`.

## The Rust core

Everything under `src-tauri/src/core/` compiles with either feature. It never names Tauri: `core/events.rs` is the one seam, and its `AppHandle` is Tauri's on the desktop and an uninhabited type on the server.

### The connection actor (`core/connection_actor/`)

Each saved connection that is in use gets one Tokio task, the `ConnectionActor` (`actor.rs`). Callers hold a cheap `ConnectionHandle` (`handle.rs`) and send it messages over a bounded mailbox. `ConnectionRegistry` (`registry.rs`) spawns actors lazily on first use, keyed by connection id, and shuts them down when a connection is removed or edited (an edited connection respawns with its new settings).

What the actor owns:

- **One persistent Redis connection**, a multiplexed `redis::aio::ConnectionManager`, opened on first use and reused by every screen. It reconnects by itself after a drop. Commands never pay a new TCP, TLS or SSH handshake, which matters on remote links.
- **The SSH tunnel**, when the connection has one, opened lazily and shared.
- **The detected BullMQ schema**, cached for the actor's lifetime.
- **The lock state** (see below).
- A **separate connection for blocking reads**: following BullMQ's event streams uses `XREAD BLOCK`, which would otherwise hold every request queued behind it.

Reads run concurrently: each one gets its own copy of the actor state and runs on a spawned task, limited to 16 at a time by a semaphore. Writes and lock changes stay in the actor loop, so they are serialised. A long job search carries a `QueryTicket` and stops early when a newer search on the same connection supersedes it.

Most handlers batch their Redis calls into pipelines to keep round trips low. `docs/slow-link.md` describes a bench for testing on a slow link.

### BullMQ schema and Lua scripts (`core/bullmq_schema/`)

This module is the only place that knows BullMQ's Redis key layout and field encoding. Feature code receives typed values (`Queue`, `Job`, `CronJob`, `FlowTree`…) and builds keys through the `Keys` trait. Do not write `"bull:` string literals anywhere else.

- **Detection.** `detect_schema` (`schema.rs`) reads the `version` field of a queue's `meta` hash (`bullmq:5.76.6`, `6.1.0`, `v5.1`) and picks `V5`, `V6` or `Unknown`. `v5.rs` and `v6.rs` hold the per-version decoders and keys.
- **Fixtures.** `fixtures/v5/` and `fixtures/v6/` hold real job hashes; `tests/bullmq_schema_versions.rs` decodes them. Supporting a new BullMQ major means adding fixtures, a decoder branch and a case in `detect_schema`.
- **Writes.** Retry, promote, add job, drain, clean, retry all failed and the scheduler upsert and removal are Lua scripts in `lua/`, ported from BullMQ's own. `_prelude.lua` holds the shared helpers (ports of the BullMQ includes of the same name). Maintenance scripts run in batches, so a queue of 200,000 jobs never blocks Redis for long and the actor can report progress and be cancelled between batches. They skip flow members, a scheduler's pending run and active jobs, and report them as skipped.
- **Reads.** `metrics.rs` (BullMQ native metrics, with an estimate when a queue records none), `failed.rs` (failures grouped by cause), `flow.rs`, `workers.rs` (`CLIENT LIST`, with a probe for managed Redis that refuses it), `timeline.rs`, `topology.rs`, `events.rs`.

### The safety lock (`core/safety_lock.rs`)

A connection is either `Locked` or `Unlocked`. Production connections are created locked (`add_connection` in `commands/connections.rs`). The actor reads the stored `is_locked` value when it spawns and keeps the live state in memory, so an unlock lasts for the session only: a restart locks again.

`check_lock_or_err` is the single enforcement point. Every destructive actor handler calls it before touching Redis. UI clicks and MCP tools go through the same handlers, so both honour the same lock. The only exception is an agent write that a human approved (see [agent approvals](#the-mcp-server-and-agent-approvals)).

### The audit log (`core/audit/`)

`core::audit::record` is the only writer to the `audit_log_entries` table; the `writer` submodule is private, so nothing else can insert. Each write sets `PRAGMA synchronous = FULL` and commits before returning, so an action the user sees confirmed is on disk. The log is append-only: there is no delete path. Entries carry the source (`Ui` or `Agent`), the target, the outcome and, for an approved agent write, the approval id. `tests/audit_durability.rs` reopens the database after each write to check it survived.

### Keychain (`core/keychain.rs`)

`KeychainAdapter` is a small trait (`set`, `get`, `delete`) with three implementations: `OsKeychain` (the `keyring` crate: macOS Keychain, Secret Service on Linux, Credential Manager on Windows), `InMemoryKeychain` for tests, and the server's `FileSecretStore`. Entries use the service name `dev.ranch.app` and the connection id as the account; the SSH password and the pinned SSH host key have their own entries (`{id}::ssh`, `{id}::ssh::hostkey`). Secrets never enter the renderer and never go into SQLite.

### SSH tunnels and TLS

- `core/ssh_tunnel.rs` opens tunnels with `russh` (pure Rust): a local listener on a random port, forwarded to Redis through the bastion, with optional jump host. The bastion's host key is pinned on first use and checked on every later connect. `core/ssh_config.rs` holds the stored tunnel settings.
- `core/tls.rs` is the only place that builds a `redis::Client`. TLS uses rustls; a custom CA is added to the system roots, not substituted for them. The CA is a public certificate and is stored in SQLite.

### SQLite and migrations (`core/db.rs`, `src-tauri/migrations/`)

`Db::init` opens `ranch.db` in the app data directory with WAL and foreign keys on, and runs the embedded `sqlx` migrations (`NNNN_description.sql`, applied in order at startup). Add a migration as a new numbered file; never edit one that has shipped. The main tables:

| Table | Holds |
|---|---|
| `groups`, `environments`, `connections` | The sidebar hierarchy. An environment owns a colour; a connection belongs to one environment. |
| `kv_settings` | App settings, including the agent settings and the GUI heartbeat. |
| `audit_log_entries` | The audit log. |
| `mcp_pending_approvals` | Agent writes waiting for a human, and recent decisions. |
| `mcp_activity_events` | Every MCP tool call. |
| `cause_notes` | Notes agents attach to a failure cause. |

The database is also the channel between the desktop app and `ranch-mcp`, which are separate processes.

### Errors

`core/errors.rs` defines `RanchError` with `thiserror`. It crosses every boundary as `RanchErrorDto` (`kind`, `message`, `docUrl`): to the front end through the generated bindings, over HTTP from the server, and as structured MCP errors. A failure is reported, never swallowed.

## Commands and the UI/MCP parity contract

### Adding a command

A command lives in a module of `src-tauri/src/commands/` as two functions:

```rust
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn retry_job(args: JobActionArgs, state: tauri::State<'_, AppState>)
    -> Result<JobActionResult, RanchErrorDto> {
    retry_job_inner(args, state.db.pool(), &state.keychain, state.connections.as_ref())
        .await
        .map_err(Into::into)
}

pub async fn retry_job_inner(/* pool, keychain, registry… */) -> Result<JobActionResult, RanchError> {
    // the actual work, shared by every shell
}
```

Then register it in three places:

1. `commands/mod.rs`, in `specta_builder()`, so Tauri and the TypeScript bindings know it.
2. `COMMAND_NAMES` in `commands/registry.rs`.
3. `server/dispatch.rs`, or `DESKTOP_ONLY` if it cannot work in a container. The test `every_ui_command_is_served_or_desktop_only` (`server/tests.rs`) fails otherwise.

### The registry (`commands/registry.rs`)

The registry lists, in plain constants:

- `COMMAND_NAMES`: every command the UI can call.
- `MCP_TOOLS`: every MCP tool, each declared `Read` or `Write`. There is no runtime guess: the declaration decides whether a tool goes through the safety gate.
- `MCP_ALIASES`: MCP names that differ from the UI command (`inspect_job` → `get_job`, `list_schedulers` → `list_cron_jobs`).
- `MCP_FORBIDDEN_COMMANDS`: commands that must never become MCP tools, because an agent could use them to lift its own restrictions or reach outside Ranch. This covers the session lock, the MCP read-only flags and safety settings, the AI switch, the approval queue and its history, the generic settings writer, the activity log, the client installers, dismissing notes, launching processes, and anything that deletes keychain entries.

The contract is asymmetric: **UI ⊇ MCP**. Every MCP tool resolves to a UI command; many UI commands (connection management, safety, settings) have no MCP twin by design.

### The tests that hold it

- `tests/parity_contract.rs` checks that every MCP tool resolves to a UI command after aliases, pins the exact MCP tool set and the command counts, so any addition is a conscious change.
- `tests/mcp_safety.rs` walks the live rmcp tool router: every exposed tool is classified, the write set is the expected one, and no forbidden command is exposed or declared.

### TypeScript bindings

`src/lib/tauri/bindings.ts` is generated by `tauri-specta` from `specta_builder()`. Every argument and result type derives `specta::Type`. The file is regenerated:

- at launch of a debug build (`pnpm tauri:dev`), and
- headlessly by `cargo test --test codegen` (in `src-tauri/`).

The file is committed, so a front-end type check needs no Rust build. Regenerate it after changing a command's signature and commit the result. Do not edit it by hand.

## The MCP server and agent approvals

`ranch-mcp` (`src-tauri/src/bin/ranch_mcp.rs`) is a separate binary that agent clients launch over stdio. The desktop app does not spawn it. It opens the same `ranch.db` and keychain entries as the app, builds its own `ConnectionRegistry`, and serves `RanchMcpServer` (`core/mcp/server.rs`, built on `rmcp`). Each `#[tool]` method converts its input and calls the matching `*_inner` function. The tool list is documented in [mcp-tools.md](mcp-tools.md).

Every call goes through `call_tool`, which:

1. refuses every tool with `aiDisabled` when AI features are off (`ai_enabled` in `core/mcp/gate.rs`);
2. runs the tool, where write tools pass the gate described below;
3. applies the job-data sharing policy to tools that return job data (`core/mcp/share.rs`: everything, sensitive field names masked as `•••` (the default), or names and errors only);
4. records the call, reads included, in the activity log (`core/mcp/activity.rs`), best effort.

### The write gate (`core/mcp/gate.rs`, `core/mcp/approval.rs`)

A write tool passes three checks in order:

1. **MCP read-only.** `connections.mcp_readonly`, or the global default when it is unset. Production connections are read-only for agents by default. Blocked calls fail with `BlockedReadonly`.
2. **Human approval**, according to the global approval mode: `off`, `writes_only` (the default) or `all` (which also holds reads on a connection). If the desktop app is not running (its heartbeat in `kv_settings` is older than 15 seconds), the call fails at once with `HostUnavailable`. Otherwise `ranch-mcp` inserts a row in `mcp_pending_approvals` with a preview of the targets and polls it every 500 ms until it is approved, denied or times out (120 seconds by default).
3. **Session lock**, inside the actor, as for a UI write.

On the desktop side, `core/mcp/watcher.rs` polls the pending table and emits `mcp:pending_approval` when the set changes; the front end shows the approval sheet; `decide_pending_approval` records the answer.

An approved call produces an `ApprovalGrant` that only `core::mcp` can construct. The actor trades it once: it marks the row `consumed` with a conditional `UPDATE`, then runs the operation even on a locked connection. Each status change (`pending → approved | denied`, `pending → expired`, `approved → consumed`) is a conditional update whose affected-row count decides the outcome, so a forged, expired, denied or already used id never authorises a write.

### Prompt injection

Job payloads, names and errors pass through as opaque JSON values. The server never interprets them as instructions.

## The front end

React 19, TypeScript, Vite, Tailwind CSS v4, TanStack Router and Query, Zustand, Base UI and Radix primitives, Lucide icons.

```text
src/
├── main.tsx, App.tsx    boot, runtime choice, providers (theme, query client, router)
├── routes/              TanStack Router file routes (routeTree.gen.ts is generated)
├── layouts/             AppShell (sidebar + screen) and Screen (toolbar, tabs, inspector)
├── features/            one folder per area
├── components/ui/       primitives (button, sheet, inputs, menus…)
├── components/data/     data components (DataTable, charts, StatePill…)
├── components/menus/    TargetMenu: one right-click menu, native or in-window
├── commands/            the command registry, menus, shortcuts
├── lib/                 bindings, runtime, polling, windows
├── stores/              Zustand stores, one per concern
├── hooks/               small shared hooks
├── i18n/                i18next, English strings in locales/en.json
└── dev/webPreview.ts    the in-memory world for Ranch web
```

### Features (`src/features/`)

Each folder owns its components, hooks and tests: `home` (all connections), `attention`, `connections` (sidebar, wizard, editing), `queues`, `jobs` (list, inspector, bulk actions, add job), `failed`, `schedulers`, `workers`, `flows`, `metrics`, `overview`, `timeline`, `map`, `audit`, `agents`, `mcp` (setup, safety, approvals, activity), `askai`, `notes`, `safety` (the lock button), `settings`, `palette`, `live`, `freshness`, `notifications`, `onboarding`, `updater`, `deep-link`.

### Routing

Routes are files under `src/routes/`; the Vite plugin regenerates `src/routeTree.gen.ts`, which is committed. The URL is the source of truth for what is on screen, with search parameters validated by Zod (`validateSearch`):

| Route | Screen |
|---|---|
| `/` | All connections |
| `/attention` | Needs attention |
| `/connections/$connectionId?view=…` | A connection: overview, queues, timeline, map, schedulers, workers, flows, failed |
| `/connections/$connectionId/queues/$queueName?state=…&job=…` | A queue: job list and inspector |
| `/connections/$connectionId/queues/$queueName/$jobId` | A job in its own window |
| `/agents?tab=…` | Approvals, activity, notes, setup |
| `/audit` | Audit log |
| `/settings?tab=…` | Settings (its own window on the desktop) |

### The command registry (`src/commands/`)

`registry.ts` lists every user action that has a place in a menu, the palette or a shortcut: its id, label, menu, section and chord. It says nothing about what the action does. The component or route that owns an action registers a handler with `useCommandHandler` (`handlers.ts`), with whether it applies now and an optional label for the context ("Resume “emails”"). No handler, or a disabled one, greys the item out.

The same registry feeds:

- the native menu bar on the desktop (`NativeMenuBar.tsx`, built with `@tauri-apps/api/menu`; the Agents menu only appears when AI features are on);
- the "Ranch" menu button in the browser runtimes (`RanchMenuButton.tsx`);
- window-wide keyboard shortcuts (`useRegistryShortcuts.ts`, chords parsed in `chords.ts`);
- the ⌘K palette (`features/palette/CommandPalette.tsx`), which shows the same chords;
- the shortcuts dialog (`ShortcutsDialog.tsx`).

Right-click menus are described as data in `components/menus/TargetMenu.tsx` and rendered as a native popup on the desktop or a Base UI menu in a browser.

### Data fetching and polling

All backend calls go through the typed `commands` object in `bindings.ts`, wrapped in TanStack Query hooks inside each feature (`useQueues`, `useJob`, `useFailedJobs`…). A command returns `{ status: "ok", data }` or `{ status: "error", error }`; hooks turn errors into thrown values that keep the typed `kind`.

Polling is shared: `src/lib/polling/sharedPoll.ts` runs one self-paced loop per query key, however many components show it. A loop runs as often as its most eager subscriber asks, never faster than three times the last read's duration (so reads space out on a slow link), and pauses while the window is hidden. Components keep `useQuery` with `refetchInterval: false` and call `useSharedPoll` with the key. A mutation can declare the data it changes (`meta.holdsPoll`) so a read does not overwrite an optimistic update. The user sets the base interval in Settings › Refresh.

On the desktop, `features/live/useConnectionEvents.ts` follows the connection on screen (`follow_connection_events`): the actor reads BullMQ's event streams and sends `queue:events`, and the queues that moved are read again at once. Polling then becomes a slow safety net. The server runtime does not follow events and keeps polling.

### Design tokens

All colours, sizes and radii are CSS custom properties in `src/styles/globals.css`, light in `:root` and dark in `.dark` (set by `next-themes`, default "system"). Components use them through Tailwind utilities such as `bg-(--surf)` or `h-control`. See [design.md](design.md).

## Testing

| What | Where | Run |
|---|---|---|
| Front-end unit and component tests | `src/**/*.test.ts(x)`, Vitest + Testing Library on jsdom | `pnpm test` (part of `pnpm check`) |
| Token contrast | `src/styles/tokens.test.ts` | `pnpm test` |
| Rust unit tests | next to the code | `cd src-tauri && cargo test` |
| Rust integration tests | `src-tauri/tests/*.rs` | `cd src-tauri && cargo test` |
| Live suites, real Redis | `src-tauri/tests/*_live.rs` | see below |
| BullMQ compatibility, real workers | `src-tauri/tests/*_compat_live.rs` | see below |
| Overflow, contrast, AI off, sheets | `e2e/`, Playwright on Ranch web | `pnpm check:overflow` |

`pnpm check` runs formatting, Biome, the type check and the unit tests. Pre-commit hooks (`lefthook.yml`) run Biome, `cargo fmt --check` and `cargo clippy -D warnings`.

### Integration and live tests

Integration tests use a temporary SQLite database and `InMemoryKeychain`. Tests that need Redis skip unless `RANCH_LOCAL_REDIS` is set:

```bash
docker compose --profile redis --profile bullmq up -d     # Redis on :6390, seeded queues
(cd .docker/bullmq-compat && npm ci)                       # bullmq 5 and 6
cd src-tauri && RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 RANCH_BULLMQ_COMPAT=1 cargo test
```

The `*_compat_live.rs` suites run real BullMQ 5.x and 6.x workers (`.docker/bullmq-compat/`) on what Ranch's Lua scripts wrote: a job Ranch added, retried or scheduled must be processed by BullMQ itself. `tests/common/mod.rs` has a slow TCP proxy for latency tests. SSH tests need `RANCH_SSH_LIVE` and a reachable SSH server. `tests/mcp_smoke.rs` calls the tool methods directly; `tests/mcp_ai_disabled.rs` drives the real `ranch-mcp` binary over stdio against a throwaway database (`RANCH_MCP_DATA_DIR`).

### Playwright checks

`pnpm check:overflow` starts (or reuses) `pnpm dev` on port 1420 and runs every project in `playwright.config.ts` against the demo world:

- **Overflow** (`e2e/overflow/`): every screen in `routes.ts`, dark and light, at 1440×900 and 1100×700. It fails on text that is clipped or ellipsised, runs into its neighbour, or leaves the window. Results go to `e2e/.overflow-report.json`. `e2e/overflow-allowlist.json` lists accepted failures; it is empty and should stay that way: fix new failures instead of listing them.
- **Contrast** (`e2e/contrast/`): every screen in both themes, text under WCAG AA fails.
- **Sheets** (`e2e/sheets/`): the sheets and the palette, for overflow and contrast.
- **AI off** (`e2e/ai/`): with AI features off, no screen shows the AI colour or the ✦ glyph.

Run `pnpm exec playwright install chromium` once before the first run. A new screen adds its route to `e2e/overflow/routes.ts`.

## Where to start for common changes

| Change | Start here |
|---|---|
| A new read of BullMQ data | `core/bullmq_schema/` for decoding, a handler in `core/connection_actor/actor.rs`, a command in `commands/`, a hook in the feature. |
| A new write | As above, plus the lock check, `core::audit::record`, and a Lua script ported from BullMQ when the operation must be atomic. |
| Exposing an operation to agents | A `#[tool]` in `core/mcp/server.rs`, an entry in `MCP_TOOLS` with its kind, and an update to `tests/parity_contract.rs` and `docs/mcp-tools.md`. |
| A new screen | A route file in `src/routes/`, a folder in `src/features/`, demo data in `src/dev/webPreview.ts`, the route in `e2e/overflow/routes.ts`. |
| A menu item or shortcut | `src/commands/registry.ts`, then a `useCommandHandler` where the action lives. |
| A setting | `kv_settings` through `get_kv_setting` / `set_kv_setting`, a section in `src/features/settings/`. |
| A database change | A new file in `src-tauri/migrations/` and a test in `tests/db_migrations.rs`. |
