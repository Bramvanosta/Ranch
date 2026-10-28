# Adding a queue backend

Ranch reaches queues through one port, so a queue system other than BullMQ on Redis is an adapter: the operations, the desktop app, Ranch server, the MCP tools and the front end stay as they are. This page says what an adapter implements and how to know it is right. The decision behind it is [ADR 0006](adr/0006-queue-backends-are-a-port.md).

## The port

Everything lives in `src-tauri/crates/application/src/ports/`:

- `backend.rs`: `Backends`, which opens a connection's backend by id and lists the kinds it serves; `BackendSpec`, one kind of backend; and `QueueBackend` with its families of methods.

What a kind can do is plain data in `src-tauri/crates/domain/src/capabilities.rs`: `Capabilities`, and `Capability`, the name of one of its flags.

Two adapters show the shape: `crates/bullmq-redis/` (the real one, forwarding to the connection actors, a crate of its own) and `adapters/fake_backend/` (in memory, short enough to read in one sitting). A backend that reaches its server is best a crate of its own too, beside it under `src-tauri/crates/`, whose manifest names the domain, the application and its client: then the compiler keeps it to its ports ([the dependency rule](architecture.md#layers-and-crates)).

## 1. Declare the kind

A connection names its backend by a kind (`bullmq-redis`) and stores where it is reached as a JSON object, its endpoint, which only the backend reads. Implement `BackendSpec` for your kind:

- `kind()` and `label()`: the id a connection stores and the name the UI shows.
- `capabilities()`: what every connection of the kind can do (see [step 5](#5-declare-capabilities)).
- `endpoint_schema()`: what the connection form asks for, field by field (`EndpointSchema`): a key, a label, a kind (text, number, boolean, secret, PEM, the SSH tunnel group), whether it is required and its default. A secret field is asked for and sent apart from the endpoint, to the secret store; it never enters the endpoint.
- `validate(endpoint)`: the endpoint as it is stored, or an `InvalidInput` saying what is wrong. Read it into a type of your own here and nowhere else: the rest of Ranch never looks inside.
- `same_server(saved, new)`: whether a saved secret may be sent to the new endpoint. A saved password is only ever sent back where it was saved for; answer `false` when in doubt.
- `tunnel(endpoint)` and `describe(endpoint)`: the SSH tunnel it goes through, if any, and one line saying where it points without credentials, for agents.
- `probe(endpoint, secrets, keychain)`: what the connection sheet's test reports before anything is saved. Reach the server once with the secrets given (`ProbeSecrets`), open nothing that outlives the call, and answer a `ProbeReport` (`crates/domain/src/probe.rs`): whether it answered, its version, why not (a `ProbeError`: refused login, unreachable, timeout, TLS, SSH tunnel), the SSH host key pinned in `keychain` on first use, and what it holds (queues, schedulers, workers). An unreachable server is a report, not an error. `test_connection` validates the endpoint with your `validate` first and refuses control characters in the secrets, then calls this.

`crates/bullmq-redis/src/endpoint.rs` is the BullMQ one: `RedisEndpoint` is the typed endpoint the connection actor reads, and `probe.rs` the probe (`PING`, `INFO server`, a bounded scan for queues).

`list_backends` sends each kind's label, schema and capabilities to the front end. The connection sheet takes its first step from `connectionForms[kind]` (`src/features/connections/connectionForms.ts`): give your kind an entry, reading and writing its endpoint through one module as `src/domain/bullmqEndpoint.ts` does for BullMQ. A connection whose kind has no entry is listed, and its sheet says it has no form.

## 2. Implement `Backends`

`specs()` lists the kinds this `Backends` serves. `open(connection_id)` returns an `Arc<dyn QueueBackend>` for a saved connection, or `ConnectionNotFound`. It resolves everything the backend needs itself (settings from `ranch.db`, secrets from the `SecretStore`), so operations only ever pass an id. Keep anything expensive (a connection pool, a client) behind it, shared between calls: `open` runs once per operation.

## 3. Implement `QueueBackend`

Required:

- `stamp()`: the connection's id, name and environment colour, as errors and audit rows name it.
- `capabilities()`: the kind's, the value its spec returns (the contract suite checks they agree).
- `connection()`: ping, and the live lock state with `lock` and `unlock`. An unlock lasts for the session; a new process starts from the saved state.
- `queues()` and `jobs()`: the queues, each with the `QueueId` you give it and its name, with their jobs per state (`StateCounts`, by state code: count the states you keep, an absent one reads as zero); job lists, one job, its logs, recent jobs, failed jobs and their causes.

Optional, each returning `Some(&dyn Family)` when provided: `job_actions()`, `maintenance()`, `queue_control()`, `schedules()`, `workers()`, `flows()`, `metrics()`, `events()`, `server()`. Leave out what the backend cannot do; the defaults return `None`.

A queue id is yours to encode: whatever names a queue in your system, as long as you can read it back from the id alone (an argument carries only the id). Nothing outside the adapter builds or takes one apart; BullMQ's is `{prefix}:{name}`, the in-memory fake's `memory/{name}`. A job's `JobRef` carries the queue's id and name.

Arguments and results are the domain's types (`src-tauri/crates/domain/`): a job is named by a `JobTarget` (its queue's id and its id), a page or a filter by a query (`JobPageQuery`, `FailedJobsQuery`, `QueueMetricsQuery`…), and nothing names the connection, which the backend was opened for. Map your system's concepts onto them: a job's state is one of the common lifecycle's (`JobState::Waiting`…) or one of your own, a `JobState::Other(BackendState { code, label, tone })` declared as a constant, a recurring job is a scheduler, a parent-child relation is a flow. The domain functions in `crates/domain/src/failures.rs`, `crates/domain/src/metrics.rs` and `crates/bullmq-redis/src/bullmq_schema/recent.rs` build the paged and grouped results from plain records; use them rather than re-deriving the rules.

## 4. Run writes through the write pipeline

Every write takes a `Caller` and follows `WritePipeline` (`crates/application/src/writes.rs`), one write at a time per connection, holding off lock changes until it is done:

1. `authorize` against the live lock state (an approved agent write passes a lock once);
2. act;
3. `record` the outcome in the audit log, then reply.

Refusals before acting (a job in the wrong state, an unknown scheduler) are not recorded. A bulk action is authorized once and recorded as one row; when the lock refuses it, that refusal is recorded too. The fake's `backend.rs` does all of this in a few lines per write.

## 5. Declare capabilities

Declare them once, as a `static` your spec's `capabilities()` and every backend's `capabilities()` return: they are known without connecting. `Capabilities` has one record per family, a flag per variant (`schedules.pause`, `maintenance.drain`, `workers.count`…), the job states the backend keeps (the common ones it uses and its own, each a `JobState::Other(BackendState { code, label, tone })`, in the order a queue shows them), and whether its metrics are its own. A state argument arrives as a code, and `Capabilities::job_state` resolves it against that list. Declare exactly what you provide: a family must be `Some` when any of its flags is on and `None` otherwise. Operations check the flag before calling, so an undeclared variant never reaches your code; the user or agent gets an `unsupported` error naming it (`schedules.list`). A backend that cannot follow events is simply not followed, and the app keeps polling.

What you declare is all the UI and the agents learn of your kind, before any connection is opened:

- The front end reads it through `src/capabilities/`: tabs, menu items, palette entries and actions you do not declare are absent, and your states are the queue's state tabs, pills and totals, with your labels and tones. The common lifecycle's headline states always have a tab; yours appear while they hold jobs.
- The MCP server lists a tool only when a kind it serves declares the capability the tool needs (its operation's in `crates/application/src/catalogue.rs`), and `list_jobs` takes the state codes the kinds describe.

Declare what the kind can do, not what one server allows: a Redis that refuses `CLIENT LIST` still declares `workers.list`, and the call says it cannot list them.

## 6. Wire it in

The composition roots choose the backend: `AppState::new` (`app_state.rs`, for the desktop app and Ranch server) and `bin/ranch_mcp.rs`. Choosing a backend per connection, rather than per process, is a `Backends` that opens one adapter or another from the saved connection.

## 7. Run the contract suite

`tests/common/backend_contract.rs` holds `run_suite(backends, seeder)`: listing and counters, single and bulk job actions, adding jobs, pausing a queue, maintenance, the scheduler lifecycle, workers, the lock and the audit rows each write leaves. It asserts only what every backend must agree on.

Give it a `Seeder` for your store (create a connection, fill a queue with jobs in given states, read the audit log back) and call it from a test file, as `tests/backend_contract.rs` (in memory) and `tests/backend_contract_live.rs` (BullMQ, ignored without Redis) do. Each section is also a public function: a backend that declares less than `run_suite` exercises calls the sections that apply to it, and what it does provide must pass unchanged.
