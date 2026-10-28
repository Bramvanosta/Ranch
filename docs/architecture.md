# Architecture

A map of the code for contributors: how the pieces fit and where to look when you want to change something. The features are in the [README](../README.md), the agent tools in [mcp-tools.md](mcp-tools.md), the visual system in [design.md](design.md), the tests in [testing.md](testing.md) and the decisions behind the shape in the [decision records](adr/README.md).

Paths: `src/…` is the front end, from the repository root. Rust paths are relative to `src-tauri/`: `crates/…` for a crate, `tests/…` for the integration tests, and a bare module path (`commands/`, `shells/mcp/server.rs`) for the app crate's `src-tauri/src/`.

## Contents

- [What Ranch is](#what-ranch-is)
- [Layers and crates](#layers-and-crates)
- [A request's path](#a-requests-path)
- [The backend port and capabilities](#the-backend-port-and-capabilities)
- [Persistence and seals](#persistence-and-seals)
- [The front end](#the-front-end)
- [Adding an operation](#adding-an-operation)
- [Where to start](#where-to-start)

## What Ranch is

Ranch is a client for BullMQ queues: one React app (`src/`) over one Rust workspace (`src-tauri/`). The same front end runs in three places: the Tauri desktop app, Ranch server (the same UI over HTTP, the Docker image) and Ranch web (a development build in a browser, against an in-memory demo world). Agents reach the same operations through `ranch-mcp`, an MCP server that agent clients start over stdio. Every operation is written once, reaches queues through a port that never names Redis, and every write of a connection's queues passes one pipeline that checks the lock (or a human's approval) and records the outcome in the audit log.

| Binary | Cargo feature | What it is |
|---|---|---|
| `ranch` | `desktop` (default) | The Tauri desktop app. |
| `ranch-mcp` | `desktop` | The MCP server agent clients start over stdio. |
| `ranch-server` | `server` | Ranch server: the UI and the operations over HTTP. |

## Layers and crates

```text
                ranch  (app crate: shells, operations, contract, core, composition roots)
                  │
      ┌───────────┼──────────────────┬───────────────────┬──────────────────────┐
      ▼           ▼                  ▼                   ▼                      ▼
ranch-secrets  ranch-sqlite  ranch-bullmq-redis   ranch-application ──────► ranch-domain
      │           │                  │                   ▲                      ▲
      └───────────┴──────────────────┴── implement its ports, name the domain ──┘
```

The workspace root is the app crate, `ranch` (library `ranch_lib`), where the Tauri CLI expects it. The inner layers and the adapters that reach the outside world through ports alone are crates under `crates/` ([ADR 0008](adr/0008-the-inner-layers-are-crates.md)). The app crate depends on all five and re-exports them as `domain`, `application`, `adapters::secrets`, `adapters::sqlite` and `adapters::bullmq_redis`.

| Layer | What it owns | May name | Never names | Held by |
|---|---|---|---|---|
| `crates/domain` (`ranch-domain`) | The types Ranch works with (queues, jobs and their states, flows, workers, schedules, metrics, connections, the hierarchy, audit entries, `Caller`, `ApprovalGrant`, `RanchError`, `Capabilities`) and the rules that need no I/O: the lock (`lock.rs`), production defaults, failures grouped by cause, schedule validation, the metrics arithmetic. | serde and serde_json, chrono, chrono-tz, croner, thiserror | specta, schemars, sqlx, redis, tauri; any other layer | the compiler (it depends on no workspace crate); its manifest, by a test |
| `crates/application` (`ranch-application`) | The catalogue of operations (`catalogue.rs`), the write pipeline (`writes.rs`) and the ports (`ports/`). | the domain, async-trait, chrono | specta, schemars, sqlx, redis, tauri; any layer above it | the compiler; its manifest, by a test |
| `crates/secrets` (`ranch-secrets`) | The `SecretStore` adapters: the OS keychain (feature `os-keychain`), an encrypted file for Ranch server (feature `file`), and an in-memory one for tests. | the application's port, the keychain, a cipher | specta, schemars; any layer but the domain and the application | the compiler; its manifest, by a test |
| `crates/sqlite` (`ranch-sqlite`) | `ranch.db`: the database and its migrations, the connection repository, the settings store, the audit writer and the approval queue's `approved → consumed` transition. | the application's ports, sqlx | specta, schemars, redis, tauri; any layer but the domain and the application | the compiler; its manifest, by a test |
| `crates/bullmq-redis` (`ranch-bullmq-redis`) | BullMQ on Redis: the connection actors, what BullMQ keeps in Redis (`bullmq_schema/`, its Lua scripts), the endpoint and the probe, the SSH tunnel and TLS. | the application's ports, redis, russh, rustls-pemfile | specta, schemars, sqlx, tauri; any layer but the domain and the application | the compiler; its manifest, by a test |
| `contract/` | The wire types: a DTO per domain type that crosses a boundary, with the specta, schemars and serde attributes the wire needs, and the `From` conversions beside it. | the domain, the application, `core`, the BullMQ schema | - | - |
| `commands/`, `server/`, `shells/` | The operations (`*_inner`) and the three shells around them. | everything below | - | - |
| `core/`, `adapters/` | The agent policy (`core/mcp/`: gate, approvals, job-data sharing, activity, heartbeat), cause notes, the MCP installer; the small adapters (the in-memory backend, the HMAC sealer, the progress sinks, the clock). | the ports, Tauri where they must | specta, schemars | a test |

A crate names only what its manifest lists and never a crate that depends on it, so a forbidden `use` in the domain, the application or an adapter crate does not compile. `tests/dependency_rules.rs` holds the rest: it checks each crate's manifest for forbidden dependencies and scans `core/` and `adapters/` for wire-format derives. `RanchError` names no store: an adapter boxes the error it met (`RanchError::database`, `RanchError::redis`).

A domain type keeps serde only where it is stored that way (an audit target is a JSON column, the SSH tunnel settings are part of a connection's JSON endpoint) and says so. Besides `contract/`, only the types a module of `commands/` declares beside its commands and the MCP tool inputs (`shells/mcp/server.rs`) derive a wire format.

## A request's path

```text
 UI click                                   MCP tool call
 src/data/<resource>/mutations.ts           agent client ─stdio─► ranch-mcp
   commands.retryJob(…)  (bindings.ts)        shells/mcp/server.rs  call_tool
     │ desktop: Tauri invoke                    ├─ AI switch (core/mcp/gate.rs)
     │ server:  POST /api/invoke/retry_job      ├─ gate: agent read-only, approval → ApprovalGrant
     ▼                                          │
 commands/jobs.rs  retry_job  (Tauri)           │
 server/dispatch.rs  routes!  (HTTP)            │
     └───────────────┬──────────────────────────┘
                     ▼
     retry_job_inner(args, backends, caller)        commands/jobs.rs
       backends.open(connection_id)                 Backends port
       require(caps, Capability::JobActionsSingle, backend.job_actions())
                     ▼
     QueueBackend::retry_job (domain types)         crates/bullmq-redis: the connection actor
       WritePipeline::authorize  ─ lock, or consume the approval once
       act (a Lua script ported from BullMQ)
       WritePipeline::record     ─ AuditLog port → crates/sqlite audit writer
                     ▼
     answer → .into() the contract's DTO → the shell → the caller
                                            MCP: job-data sharing, activity log
```

**Shells.** A shell speaks one client's protocol and nothing else. The Tauri command (`#[tauri::command]` in `commands/`), the HTTP route (`server/dispatch.rs`, `POST /api/invoke/{command}`) and the MCP tool (`#[tool]` in `shells/mcp/server.rs`) each convert their input and call the same `*_inner` function with the backends and a `Caller`: `Caller::User` from the UI, an agent caller (its client name, and the grant of an approval it holds) from MCP.

**The operation.** An `*_inner` function in `commands/` reads the contract's arguments into the domain's query (`args.into()`), opens the connection's backend, checks the capability its catalogue entry names, calls the port and converts the answer back (`.into()`), so every shell gets the same shape. Errors cross as `RanchErrorDto` (`contract/errors.rs`: `kind`, `message`, `docUrl`): to the front end through the bindings, over HTTP, and as structured MCP errors with the kind in `data.kind`.

**The catalogue.** `crates/application/src/catalogue.rs` lists every operation once, as plain data: its name; its kind, `Read` or `Write` with its trail (`Pipeline` for a write of a connection's queues, or `Outside` with the reason: Ranch's own records, the lock toggles, a note on a cause, what acts outside Ranch); the capability it needs; whether the HTTP server serves it or refuses it with a `desktopOnly` message; and what the MCP server does with it: a `Tool` of its name or `ToolNamed` another (`get_job` is `inspect_job`), `UiOnly` with why, or `Forbidden` with why (anything an agent could use to lift its own limits or reach outside Ranch). The kind decides whether a tool passes the safety gate; the capability whether it is listed. Every operation is a UI command, so agents never do more than the UI ([ADR 0003](adr/0003-agents-never-do-more-than-the-ui.md)). The shells keep the literal lists their macros need, and a test of each fails when its list and the catalogue differ ([testing.md](testing.md#the-shells-held-to-the-catalogue)).

**The MCP gate.** `call_tool` refuses every tool with `aiDisabled` when AI features are off, runs the tool, applies the job-data sharing policy to tools that return job data (`core/mcp/share.rs`), and records the call in the activity log (`core/mcp/activity.rs`), best effort. A write tool first passes the gate (`core/mcp/gate.rs`, `core/mcp/approval.rs`):

1. **Agent read-only**: the connection's flag, or the global default. Refused with `blockedReadonly`.
2. **Approval**, by the approval mode (`off`, `writes_only` by default, or `all`, which also holds reads on a connection). Without a recent heartbeat from the desktop app the call fails at once with `hostUnavailable`. Otherwise `ranch-mcp` inserts a row in `mcp_pending_approvals` and polls it until it is approved, denied or times out. On the desktop, `core/mcp/watcher.rs` emits `mcp:pending_approval` and the front end shows the approval sheet; `decide_pending_approval` records the answer, sealed. Once the app's seal over the approved call checks out, the gate mints an `ApprovalGrant` for it.
3. **The lock**: a write that needs no approval is refused on a connection the app sealed as locked, as the database holds it now; then the write pipeline checks the actor's lock, as for a UI write.

**The write pipeline.** `WritePipeline` (`crates/application/src/writes.rs`) holds the rule every write of a connection's queues follows: authorize (the connection's live lock, or a grant consumed once for this connection and tool), act, record the outcome in the audit log before replying. The connection actor runs the three steps in one turn of its write loop, so a lock change never lands between the check and the write ([ADR 0005](adr/0005-the-write-pipeline-runs-in-the-write-loop.md)). An approved agent write runs even on a locked connection: the approval is the human's gesture for that one operation. `ApprovalGrant::new` is public ([ADR 0008](adr/0008-the-inner-layers-are-crates.md)), so the guarantee is not who builds a grant but what it is traded against: the pipeline consumes it through `ApprovalStore::consume`, a conditional `approved → consumed` update on the approved, sealed row for this connection and tool, and a forged, expired, denied, already used or mismatched id authorizes nothing. Shells and operations never authorize or audit a queue write themselves.

**The connection actor.** The BullMQ backend's engine (`crates/bullmq-redis/src/connection_actor/`, [ADR 0001](adr/0001-one-actor-per-connection.md)). Each connection in use gets one Tokio task, spawned lazily by the `ConnectionRegistry` and reached through a cheap `ConnectionHandle`. It owns one persistent Redis link (reconnected after a drop), the SSH tunnel, the detected BullMQ schema, the live lock state and a separate link for blocking stream reads. Reads run concurrently on copies of its state (16 at a time); writes and lock changes run in its loop, one at a time, each through `writes/authorize.rs` and the write pipeline. Writes that move or create jobs are Lua scripts in `bullmq_schema/lua/`, ported from BullMQ's own ([ADR 0002](adr/0002-writes-are-ports-of-bullmq-scripts.md)); `bullmq_schema/` is the only place that knows BullMQ's key layout (no `"bull:` literal elsewhere). [slow-link.md](slow-link.md) has a bench for a remote Redis.

**Progress.** What a backend reports while it works (bulk and maintenance progress, a followed connection's queue events) goes to the `ProgressSink` port: the desktop's adapter (`adapters/progress.rs`) emits window events, the other shells drop them.

## The backend port and capabilities

Operations reach queues only through `Backends` and `QueueBackend` (`crates/application/src/ports/backend.rs`, [ADR 0006](adr/0006-queue-backends-are-a-port.md)). `Backends::open(id)` returns a connection's backend, resolving its settings and secrets itself; `QueueBackend` hands out families of methods: queues and jobs always; job actions, maintenance, queue control, schedules, workers, flows, metrics, events and server reads optionally. Arguments and answers are domain types, and nothing names the connection the backend was opened for.

A connection is a **backend kind** (`bullmq-redis`) and that kind's **endpoint**, JSON only the backend reads. Each kind has a `BackendSpec`: its label, its `EndpointSchema` (what the connection form asks for), how an endpoint is validated, the probe the connection test runs, and its `Capabilities` (`crates/domain/src/capabilities.rs`): a flag per variant, each named by a `Capability` (`schedules.pause`), and the job states it keeps, each a code, a label and a tone. An operation checks the flag before calling, and a backend that does not declare it answers `unsupported` naming it, without being called. `list_backends` sends each kind's descriptor (`contract/capabilities.rs`) to the front end, and the MCP tool list follows the same declaration. What one server refuses at run time (a managed Redis without `CLIENT LIST`) stays an error or an empty state of the call.

Two adapters implement the port: `crates/bullmq-redis/` and `adapters/fake_backend/` (in memory, for tests). The composition roots pick them: `AppState::new` (`app_state.rs`) for the desktop app and Ranch server, `bin/ranch_mcp.rs` for the MCP server. [adding-a-backend.md](adding-a-backend.md) is the guide for another queue system.

The other ports, all in `crates/application/src/ports/`:

| Port | What it is | Adapter |
|---|---|---|
| `AuditLog`, `ApprovalStore` | Where the write pipeline records outcomes and trades approvals. | `crates/sqlite/`; in memory in `adapters/fake_backend/` |
| `ConnectionRepository` | Saved connections, environments and groups. A write runs the keychain step it is given inside its transaction, so a refused secret rolls the row back. | `crates/sqlite/src/connections.rs` |
| `SettingsStore` | The key/value settings. | `crates/sqlite/src/settings.rs` |
| `SecretStore` | A connection's passwords and pinned SSH host key. | `crates/secrets/` |
| `Sealer` | Makes and checks seals, one per process. | `adapters/hmac_sealer.rs` |
| `ProgressSink` | Bulk progress and queue events, and whether anything shows them. | `adapters/progress.rs` |
| `Clock` | The time the stores write. | `adapters/system_clock.rs` |

## Persistence and seals

**SQLite.** `Db::init` (`crates/sqlite/src/db.rs`) opens `ranch.db` in the app data directory with WAL and foreign keys on, and runs the embedded `sqlx` migrations (`crates/sqlite/migrations/`, applied in order at startup). Add a migration as a new numbered file; never edit one that has shipped (sqlx checksums them). The tables: the hierarchy (`groups`, `environments`, `connections`, each connection with its `backend` and `endpoint`), `kv_settings` (settings, the agent settings, the desktop app's heartbeat), `audit_log_entries`, `mcp_pending_approvals`, `mcp_activity_events` and `cause_notes`. The database is also the channel between the desktop app and `ranch-mcp`, which are separate processes.

**The audit log.** `ranch_sqlite::audit::record` is the only writer to `audit_log_entries`: the writer module is private. It sets `PRAGMA synchronous = FULL` and commits before returning, so a write the user sees confirmed is on disk. There is no delete path. Queue writes reach it through the write pipeline's `AuditLog` port; the lock toggles (`commands/safety.rs`) call it directly. Entries carry the source (`Ui` or `Agent`), the target, the outcome and, for an approved agent write, the approval id.

**Secrets.** Passwords and the pinned SSH host key live in the `SecretStore`, never in SQLite or the renderer: the OS keychain on the desktop (service `dev.ranch.app`, the connection id as account; `{id}::ssh` and `{id}::ssh::hostkey` for the tunnel), an encrypted file in Ranch server. Each shell enables the store it needs, so the server links no keychain and the desktop app no cipher. A TLS CA is a public certificate and is part of the endpoint.

**The lock.** A connection is `Locked` or `Unlocked` (`crates/domain/src/lock.rs`). Production connections are created locked and read-only for agents (`crates/domain/src/production.rs`). The actor reads the stored state when it spawns and keeps the live one in memory, so an unlock lasts for the session.

**Seals.** Any process of the user can write `ranch.db`, so `ranch-mcp` does not take its word for what an agent may do ([ADR 0004](adr/0004-ranch-mcp-trusts-seals-not-the-database.md)). The app seals, with an HMAC whose key is in the OS keychain (`ranch::integrity`), each approval it grants, the safety settings (AI switch, approval mode, read-only default, job-data sharing) and each connection's agent read-only flag, lock, backend and endpoint; `ranch-mcp` reads a missing or wrong seal as the strictest value, and opens no connection whose seal does not hold, reads included (`connectionUnverified`): the row says where the password goes. A connection's password is the keychain entry named after its id (`password_entry`), never one the database names. A run of the app with an existing key carries over the connection seals made before they covered the endpoint, those that hold (`ConnectionRepository::upgrade_seals`). The stores seal, not their callers ([ADR 0007](adr/0007-seals-are-made-inside-the-stores.md)): `SettingsStore` seals a safety setting on every write and returns it with its verdict through `safety`, and `ConnectionRepository` seals a connection's flags and endpoint in the transaction that changes them. The `Sealer` is built per process: `HmacSealer::for_app` in the desktop app, `for_agents` in `ranch-mcp`, `unsealed` in Ranch server and on a debug build's throwaway database.

## The front end

React 19, TypeScript, Vite, Tailwind CSS v4, TanStack Router and Query, Zustand, Base UI and Radix primitives.

```text
src/
├── main.tsx, App.tsx     boot, runtime choice, providers
├── routes/               TanStack Router file routes (routeTree.gen.ts is generated and committed)
├── layouts/              AppShell (sidebar + screen) and Screen (toolbar, tabs, inspector)
├── features/<area>/      screens and feature hooks, used from outside through index.ts
├── data/<resource>/      keys.ts, queries.ts, mutations.ts: the only callers of the backend commands
├── capabilities/         what a connection's backend can do: useCapabilities, IfCapable, useJobStates
├── domain/               pure modules: job states, state counts, the BullMQ endpoint, formatting, triage
├── backend/fake/         the in-memory backend of Ranch web and the tests, one handler table per resource
├── components/           ui/ primitives, data/ tables and charts, menus/ the right-click menu
├── commands/             the command registry behind menus, shortcuts and ⌘K
├── lib/                  tauri/bindings.ts (generated), runtime/, polling/, windows/
├── stores/, hooks/       Zustand stores, small shared hooks
└── test/                 test helpers: factories, providers, results, chords
```

**Runtimes.** `src/main.tsx` picks the runtime at boot (`src/lib/runtime/runtime.ts`). On the desktop, commands go through Tauri's `invoke`. In Ranch server, `src/lib/runtime/tauriShim.ts` stands in for `window.__TAURI_INTERNALS__` and `httpBackend.ts` turns each command into `POST /api/invoke/{command}`. In Ranch web (`pnpm dev` in a browser) the shim answers from `src/backend/fake/`, a stateful BullMQ world typed against the bindings and loaded by a dynamic `import()` behind `import.meta.env.DEV`, so it never ships. Components ask what the runtime offers (`hasAgents()`, `hasNativeWindows()`, `hasLiveEvents()`…), never which runtime it is.

**Bindings.** `src/lib/tauri/bindings.ts` is generated by `tauri-specta` from `specta_builder()` (`commands/mod.rs`): the contract's types and a typed `commands` object. It is regenerated by a debug launch (`pnpm tauri:dev`) or headlessly by `cargo test --test codegen`, and committed, so a type check needs no Rust build. Never edit it by hand.

**Features.** One folder per area (`home`, `attention`, `connections`, `queues`, `jobs`, `failed`, `schedulers`, `workers`, `flows`, `metrics`, `overview`, `timeline`, `map`, `audit`, `agents`, `mcp`, `settings`, `palette`, `live`…), each exposing what others may use in its `index.ts`. `src/features/boundaries.test.ts` fails on an import of another feature's internals, on a shared layer importing a feature, on a circle between features, and on a feature or route importing the bindings' `commands`.

**Data.** `src/data/<resource>/` holds the query key factories, queries and mutations of one resource. `unwrap()` (`src/data/result.ts`) turns an error result into a thrown `RanchError` that keeps its `kind`. Mutation hooks own their invalidation and failure toasts. Polling is shared (`src/lib/polling/sharedPoll.ts`): one self-paced loop per query key, as often as its most eager subscriber asks, never faster than three times the last read's duration, paused while the window is hidden. On the desktop, `features/live/` follows the connection on screen through BullMQ's event streams and polling becomes a safety net. Settings are declared once in `src/data/settings/settings.ts` and read with `useSetting`.

**Capabilities.** Screens ask `src/capabilities/` what a connection's backend declares: `useCapabilities(id).can("schedules.pause")` or `<IfCapable need="…">` (an undeclared tab, menu item, card or action is absent, not disabled), and `useJobStates(id)` for the described states. `src/domain/jobStates.ts` keeps pure helpers and owns no list of states; a queue's `counts` are keyed by state code and read through `countOf` (`src/domain/stateCounts.ts`). `src/domain/bullmqEndpoint.ts` is the one module that knows the BullMQ endpoint's shape, and the connection sheet takes its form from `connectionForms[backend]` (`src/features/connections/connectionForms.ts`). Hiding is a convenience: the core refuses an undeclared capability whatever the UI shows.

**Routes.** The URL is the source of truth for what is on screen, with search parameters validated by Zod:

| Route | Screen |
|---|---|
| `/` | All connections |
| `/attention` | Needs attention |
| `/connections/$connectionId?view=…` | A connection: overview, queues, timeline, map, schedulers, workers, flows, failed |
| `/connections/$connectionId/queues/$queueId?state=…&job=…` | A queue: job list and inspector |
| `/connections/$connectionId/queues/$queueId/$jobId` | A job in its own window |
| `/agents?tab=…` | Approvals, activity, notes, setup |
| `/audit` | Audit log |
| `/settings?tab=…` | Settings (its own window on the desktop) |

A queue id only comes from a listing or the URL, never built (`useRouteQueue`, in `features/queues`); an address that names the queue instead lands on it and is replaced by its id.

**Commands and menus.** `src/commands/registry.ts` lists every user action that has a menu item, a palette entry or a shortcut. The component that owns an action registers a handler with `useCommandHandler` (`handlers.ts`); no handler greys the item out. The same registry feeds the native menu bar, the in-window Ranch menu, the shortcuts, the ⌘K palette and the shortcuts dialog. Colours, sizes and radii are CSS custom properties in `src/styles/globals.css` ([design.md](design.md)).

## Adding an operation

1. **Its DTOs** in `contract/` (re-exported by `commands`), with the `From` conversions to the port's domain query and from its answer.
2. **The operation** in a module of `commands/`: an `*_inner` function and the Tauri command beside it.

   ```rust
   #[cfg(feature = "desktop")]
   #[tauri::command]
   #[specta::specta]
   pub async fn retry_job(args: JobActionArgs, state: tauri::State<'_, AppState>)
       -> Result<JobActionResult, RanchErrorDto> {
       retry_job_inner(args, state.backends.as_ref(), Caller::User)
           .await
           .map_err(Into::into)
   }

   pub async fn retry_job_inner(args: JobActionArgs, backends: &dyn Backends, caller: Caller)
       -> Result<JobActionResult, RanchError> {
       let backend = backends.open(&args.connection_id).await?;
       require(backend.capabilities(), Capability::JobActionsSingle, backend.job_actions())?
           .retry_job(args.into(), caller)
           .await
           .map(Into::into)
   }
   ```

3. **Its entry in the catalogue** (`crates/application/src/catalogue.rs`): kind, capability, what the HTTP and MCP servers do with it, with the reason when they do not serve it.
4. **The shells' lists**, each held to the catalogue by a test:
   - `collect_commands!` in `specta_builder()` (`commands/mod.rs`): `the_tauri_commands_are_the_catalogues_operations` (`tests/parity_contract.rs`);
   - a route in `routes!` (`server/dispatch.rs`) when the server serves it: `the_routes_are_the_catalogues_served_commands` (`server/tests.rs`);
   - a `#[tool]` method and its input in `shells/mcp/server.rs` when it is a tool: `tools_list_with_every_capability_is_the_catalogues_tools` (`tests/mcp_tool_schemas.rs`). The server's instructions list it by themselves. A write tool also needs its case in `tests/mcp_safety.rs`, `tests/lock_sentinel_live.rs` and `tests/mcp_writes_live.rs`. Regenerate the tool snapshot and update [mcp-tools.md](mcp-tools.md).
5. **The bindings**, regenerated (`cargo test --test codegen`), and the front end's call in `src/data/<resource>/`.

`tests/parity_contract.rs` pins the counts and the tool set, so the change shows there too. A new backend method also goes on its family in `crates/application/src/ports/backend.rs`, with its capability, in both adapters and in the backend contract suite.

## Where to start

| Task | Files |
|---|---|
| A new read of BullMQ data | decoding in `crates/bullmq-redis/src/bullmq_schema/`, a handler in `crates/bullmq-redis/src/connection_actor/reads/` and its `ActorCommand` in `actor.rs`, a method on a family in `crates/application/src/ports/backend.rs` with its capability, forwarded in `crates/bullmq-redis/src/lib.rs` and kept in `adapters/fake_backend/`, then [an operation](#adding-an-operation) and a hook in `src/data/`. |
| A new write | as above, with the handler in `crates/bullmq-redis/src/connection_actor/writes/` going through the write pipeline, a Lua script ported from BullMQ when it must be atomic, and a section in `tests/common/backend_contract.rs`. |
| Another queue system | [adding-a-backend.md](adding-a-backend.md). |
| Exposing an operation to agents | its MCP exposure in `crates/application/src/catalogue.rs`, a `#[tool]` in `shells/mcp/server.rs`, the tests [above](#adding-an-operation), [mcp-tools.md](mcp-tools.md). |
| A domain rule | `crates/domain/src/`, unit-tested beside it. |
| The agent safety rules | `core/mcp/gate.rs`, `core/mcp/approval.rs`, `core/mcp/share.rs`; seals in `adapters/hmac_sealer.rs` and the stores. |
| A server-only behaviour | `server/` (auth in `oidc.rs` and `session.rs`, configuration in `config.rs`); [server.md](server.md). |
| A new screen | a route in `src/routes/`, a folder in `src/features/` with its `index.ts`, its reads and writes in `src/data/`, demo data in `src/backend/fake/`, the route in `e2e/overflow/routes.ts`. |
| A menu item or shortcut | `src/commands/registry.ts`, then `useCommandHandler` where the action lives. |
| A setting | a `defineSetting` in `src/data/settings/settings.ts`, `useSetting` where it is read, a section in `src/features/settings/`. |
| A database change | a new file in `crates/sqlite/migrations/` and a test in `tests/db_migrations.rs`. |
| A test | [testing.md](testing.md). |
