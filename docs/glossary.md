# Glossary

The words Ranch uses, in the code, the UI and these docs, and what each one means here.

## Where Ranch runs

- **Ranch desktop**: the Tauri app (`ranch`). The only runtime with the OS keychain, the approval sheet and `ranch-mcp`.
- **Ranch server**: the same UI served over HTTP by `ranch-server` (the Docker image). See [server.md](server.md).
- **Ranch web**: the UI in a browser during development, against an in-memory demo world; nothing reaches Redis.
- **`ranch-mcp`**: the MCP server an agent client starts over stdio. It shares the app's database and keychain entries.

## What Ranch manages

- **Connection**: a backend kind and where it is reached (its endpoint), with its passwords kept in the secret store (the OS keychain on the desktop). Every connection sits in one group and one environment.
- **Backend kind**: the kind of queue system a connection is served by, named by a string the connection stores (`bullmq-redis`: BullMQ on Redis). A kind says what its endpoint holds, checks it, probes the server behind it and declares its capabilities (`BackendSpec`).
- **Endpoint**: where a connection's backend is reached, as that backend describes it: for BullMQ on Redis, the host, port, database, user, TLS with an optional CA, and an optional SSH tunnel. Stored as JSON; only the backend reads inside it. Secrets are never part of it.
- **Group**: a folder of connections in the sidebar, usually one product or team.
- **Environment**: the stage a connection serves (Local, Staging, Production…), with its colour. Production changes the defaults: locked at start, read-only for agents.
- **Queue**: a queue of the connection's backend (on BullMQ, a BullMQ queue). It is listed with its **queue id** and its name.
- **Queue id**: how operations, URLs and agents name a queue. Opaque: the backend that listed the queue made it, and nothing else builds one or takes it apart. The BullMQ backend makes `{prefix}:{name}` (`bull:emails`), the base of the queue's Redis keys.
- **Job state**: where a job is. A job is in one state at a time. Every backend shares the lifecycle waiting, active, completed, failed, delayed and paused; a backend adds states of its own, described by a code, a label and a tone (BullMQ: prioritized, waiting children, stalled). A state crosses the wire as its code (`waitingChildren`); BullMQ tells which state a job is in by the Redis list or set its id is in.
- **Scheduler**: a BullMQ job scheduler (cron pattern or fixed interval) that produces jobs on a queue.
- **Flow**: a tree of parent and child jobs; a parent waits for its children.
- **Worker**: a process consuming a queue, as Redis reports it.
- **Cause**: failed jobs grouped by the shape of their error, so 4,000 failures read as three problems. An agent can attach a **cause note** to one.
- **Maintenance**: an operation on a whole queue: drain, clean finished jobs, retry every failed job.

## Safety

- **Lock**: a connection is locked (read-only) or unlocked. The lock applies to everyone; Production starts locked, and an unlock lasts until the app restarts.
- **Agent read-only**: a per-connection switch that refuses every agent write, whatever the lock. On by default for Production.
- **Approval**: a human's yes in the Ranch window to one agent write. The **approval mode** says which agent calls need one: none, writes, or everything. An approved call carries an **approval grant**, which the write pipeline trades once against the approved row.
- **Seal**: an HMAC the app puts on what governs agents (approvals, safety settings, agent read-only, lock, where a connection points). `ranch-mcp` reads a missing or wrong seal as the strictest value.
- **Caller**: who asks for a write, the user or an agent (with its client name and, perhaps, an approval). The audit log names them.
- **Audit log**: every write and its outcome, who did it, on which connection. Written before the write is reported as done.
- **Activity log**: every agent tool call, reads included. Best effort: a failure to log never fails the call.

## In the code

- **Queue backend**: what operations reach a connection's queues through (`QueueBackend`, opened by `Backends`). BullMQ on Redis is one; an in-memory one serves the tests.
- **Capability**: something a kind of backend declares it can do (`schedules.pause`, `maintenance.drain`…), known without connecting. An operation the backend does not declare fails with `unsupported`, naming it. The **capability descriptor** is the same declaration on the wire, with the job states the kind keeps described (code, label, tone), listed by `list_backends`: the screens (`src/capabilities/`) and the MCP tool list follow it. Not to be confused with what the runtime offers (native windows, live events), which the front end asks of `src/lib/runtime/`.
- **Connection actor**: the task that owns one connection's Redis link, the engine of the BullMQ backend. Reads run concurrently on it; writes run one at a time.
- **Operation**: what Ranch can do, written once as an `*_inner` function in `src-tauri/src/commands/`; the desktop command, the HTTP route and the MCP tool all call it.
- **Catalogue**: the one list of operations (`src-tauri/crates/application/src/catalogue.rs`, in `ranch-application`): for each, whether it reads or writes, the capability it needs, whether Ranch server serves it, and whether it is an MCP tool, left to the UI or forbidden to agents, with the reason. Each shell is tested against it.
- **Write pipeline**: the rule every write of a connection's queues follows (`WritePipeline`): authorize against the lock or an approval, act, record the outcome in the audit log before replying.
- **Port** and **adapter**: a port is an interface the application reaches the outside world through (`Backends`, `AuditLog`, `SecretStore`…); an adapter implements it (BullMQ on Redis, the SQLite stores, the OS keychain).
- **Contract**: the arguments and results of operations as they cross a boundary (`src-tauri/src/contract/`); `src/lib/tauri/bindings.ts` is generated from it. Its types mirror the domain's, which never derive a wire format.
- **Shell**: a way in that speaks one client's protocol: the Tauri commands, the HTTP server, the MCP server.
- **Parity**: agents can do what the UI does, never more; `tests/parity_contract.rs` checks it.
