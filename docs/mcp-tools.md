# Ranch MCP Tools

Ranch exposes a built-in **Model Context Protocol** server (`ranch-mcp`) that
agents like Claude Desktop and Cursor can invoke via stdio. The server shares
the same SQLite database and OS keychain as the Ranch desktop app, so an agent
sees the connections you've configured in the UI.

## Connecting a client

**One click**, from *Agents › Setup & safety* (or the first-run wizard):

| Client | How Ranch installs it |
|---|---|
| Claude Desktop | merges `mcpServers.ranch` into `claude_desktop_config.json` |
| Cursor | merges into `~/.cursor/mcp.json` |
| Windsurf | merges into `~/.codeium/windsurf/mcp_config.json` |
| Claude Code | runs `claude mcp add --scope user ranch <ranch-mcp>` |
| Codex | runs `codex mcp add ranch -- <ranch-mcp>` |

JSON merges are non-destructive (other servers and keys are kept), atomic
(temp file + rename) and refuse a config that is not valid JSON. Claude Code
and Codex own their config formats, so Ranch asks *their* CLI to register the
server — with a fixed argument vector, never a shell string. The CLI is looked
up on `PATH` and in the usual install locations (a macOS GUI app does not
inherit your shell's `PATH`); when it is not found the card says so and shows
the exact command to run yourself.

**By hand**, for any other MCP client — add this to its config, e.g.
`~/Library/Application Support/Claude/claude_desktop_config.json` on macOS:

```json
{
  "mcpServers": {
    "ranch": {
      "command": "/Applications/Ranch.app/Contents/MacOS/ranch-mcp"
    }
  }
}
```

Adjust the path to wherever your `ranch-mcp` binary lives.

## Tools

**29 tools.** Every one maps to something a human can do in the Ranch UI
(`UI ⊇ MCP`); form helpers, connection management and every safety control are
intentionally UI-only.

### Read (13) — always allowed

| Tool | Purpose |
|---|---|
| `list_connections` | Every saved Redis connection, each with its `groupId` / `groupName`, `environmentId` / `envLabel` / `envColor`, and `mcpReadonly`. |
| `list_environments` | Every environment (e.g. local, staging, production) in sidebar order, each with its `groupId` / `groupName` — typically one group per project, one environment per stage. |
| `list_queues` | BullMQ queues on a connection + state counters. |
| `list_jobs` | Paginated job list per `(queue, state)`. Optional `search` (job id or name), `reason` (failure reason), `sortBy` (`name` \| `queuedAt` \| `duration`) + `sortDescending` run across the whole state — the newest 10,000 jobs — not just the returned page; `total` is then the number of matching jobs and `scanCapped` says whether older jobs were left out. Jobs carry `progress` (raw JSON of `job.updateProgress()`). Rows are **summaries** — see below. |
| `inspect_job` | Everything about one job: payload, options, return value, progress, the whole `failedReason` and its stacktraces. BullMQ keeps one trace per attempt; identical ones are returned once, and `stacktraceIdenticalAttempts` says how many attempts produced it. |
| `list_failed_jobs` | **Everything that failed on a connection, across all its queues**, newest first: the jobs (paged, `limit` ≤ 250), per-queue counts, and the most failing job types. Optional filters `queue`, `name`, `reason` (substring). Looks at the newest 500 failed jobs per queue — "what failed on staging?" is one call. |
| `get_job_logs` | A page of a job's BullMQ log lines (`offset` default 0, `limit` default 100, capped at 500). |
| `list_schedulers` | Schedulers (CRON / repeatable jobs) of a queue: key, pattern, next run, paused. |
| `get_flow` | The flow (BullMQ `FlowProducer` parent / child tree) a job belongs to, as nested JSON from the root — per job: queue, id, name, state, duration, failure reason, children — across queues. "Why is this parent stuck in `waitingChildren`?" = the children that are not `completed`. A job in no flow returns a tree of one; cut at 200 jobs (`truncated`). |
| `list_workers` | BullMQ workers attached to a queue — id, name, address, `ageSecs`, `idleSecs`, last command — read from `CLIENT LIST`. `workers: []` = nothing is consuming the queue. `supported: false` = this Redis provider forbids `CLIENT LIST` (managed Redis, ACL), which says nothing about the workers. |
| `get_queue_metrics` | Metrics of one queue over a window (`range`: `5m`, `15m`, `30m`, `1h`, `6h`, `24h` default, `3d`, `7d`): counts and per-bucket series, throughput, failure rate, processing time and queue delay (avg / min / max), the 10 slowest jobs and the 10 most failing job types. `nativeQueues` / `totalQueues` tell whether the counts are exact (the queue's Worker records BullMQ metrics) or estimated from the jobs still in Redis. |
| `get_connection_metrics` | The same, merged over every queue of a connection. |
| `list_failure_causes` | The failures of a connection (optional `queue`) **grouped by cause** — the reason with its values replaced, as the Failed tab shows them — most frequent first: per cause its `signature`, a sample reason, the count, first and last time, job names, queues, up to 20 of its jobs, and `noted` when a note already explains it. The signature is the key for `annotate_cause`. |

With approval mode `all`, the eight tools that target a connection are held
like writes; `list_connections` and `list_environments` always stay free.

#### A list is for choosing, `inspect_job` is for reading

A row of `list_jobs` / `list_failed_jobs` carries what it takes to pick a job:
its `queueRef` + `jobId` (in the exact shape the other tools take, so a row can
be handed straight to `inspect_job` or `retry_job`), name, state, timing, the
payload, and a `failedReason` capped at 800 characters
(`failedReasonTruncated: true` when it was cut). It carries **no stacktrace and
no `opts`** — `stacktraceEntries` says how many traces the job holds, and
`inspect_job` gives them.

This is a deliberate budget. Before it, `list_failed_jobs` with `limit: 5`
answered with 35 KB — around 8,700 tokens — of which 80% was stacktraces, and
14.5 KB was the *same* trace repeated once per attempt (BullMQ stores one entry
per attempt, so a job that failed three times on one bug stores it three
times). The same call now answers in 5 KB, and `inspect_job` on the worst of
those jobs in 4.8 KB instead of 11 KB.

Paging is optional everywhere it has an obvious default: `list_jobs` and
`get_job_logs` take `offset` (0) and `limit` (50 / 100), and `list_jobs` takes
`newestFirst` (true). Only what cannot be guessed — the connection, the queue,
the state — is required. `queueRef`'s two fields are **inlined** in every
tool's schema rather than `$ref`'d, because some clients hand the agent a
`$ref`'d subschema as `{}`.

`data`, `opts`, `returnValue` and `progress` are returned as **JSON values**,
not as strings containing JSON: BullMQ stores them encoded, and an agent should
not pay for the escaping and then parse it itself. A field that does not parse
is passed through as the string it is, never dropped.

### Write (16) — behind the safety gate

| Tool | Purpose |
|---|---|
| `retry_job` | Move a failed job back to `waiting`. |
| `remove_job` | Delete a job from every state list + hash + logs. |
| `promote_job` | Move a delayed job into `waiting` immediately. |
| `bulk_retry_jobs` / `bulk_remove_jobs` / `bulk_promote_jobs` | The same for up to 1,000 job ids of one queue, as **one operation and one approval**. Returns per-job outcomes. `bulk_retry_jobs` also takes `allFailed: true` (instead of `jobIds`): every failed job of the queue, however many — the approval states the count. |
| `drain_queue` | Remove every job that has **not started**: waiting, paused, prioritized and — unless `includeDelayed: false` — delayed. Active, completed and failed jobs stay. The pending run of a job scheduler and members of a flow are left in place (`skipped`). Cannot be undone. |
| `clean_queue` | Remove the `completed` or `failed` jobs (`state`) that finished more than `olderThanMs` ago (`0` = all). Members of a flow are left in place. Cannot be undone. |
| `pause_queue` / `resume_queue` | Stop / restart job pickup on a queue. Idempotent. |
| `pause_scheduler` / `resume_scheduler` | Stop / restart one scheduler (`schedulerKey` from `list_schedulers`). Pausing removes its pending job (nothing fires) and keeps it listed, flagged `paused`; resuming re-arms it from its stored settings. |
| `upsert_scheduler` | Create a job scheduler, or change an existing one (same `schedulerId`): `jobName` + either a cron `pattern` (5 fields, or 6 with seconds first; numbers, `*`, lists, ranges, steps, month / weekday names; optional IANA `tz`) **or** `everyMs` (≥ 1000), and an optional JSON `data` template. Returns `nextRunMs` and the pending job's id. The approval shows the schedule, the next run and the template; an invalid pattern or timezone is refused before anyone is asked. |
| `remove_scheduler` | Delete a job scheduler and its pending job. A job of it that is running finishes; nothing further is produced. |
| `annotate_cause` | Attach a **resolution note** to a failure cause: `queueRef`, `signature` (from `list_failure_causes`), `text` (what is wrong, ≤ 4,000 characters), `steps` (what to do, ≤ 20), `jobsRead`. Shown next to the cause in Ranch and under Agents › Notes, with the agent client and the time. **No approval**: a note changes no queue state; it is refused on a connection read-only for agents and, like every tool, when AI features are off. |
| `add_job` | Enqueue a job on an **existing** queue: `name`, `data` (any JSON), optional `delayMs`, `attempts`, `priority` (1 = highest), `lifo`. Returns `jobId` and the `state` it landed in (`waiting`, `paused`, `delayed`, `prioritized`). The approval preview shows the queue, the name, the options and the whole payload. An invalid job or an unknown queue is refused before anyone is asked. |

**One call = one approval.** That is why the bulk tools exist on this surface:
"remove these 12 jobs" is one decision for the human — with the count and the
first job names in the approval preview — not twelve modals.

Example (`bulk_remove_jobs`):

```json
{
  "connectionId": "0198…",
  "queueRef": { "prefix": "bull", "name": "report-generation" },
  "jobIds": ["4101", "4102", "4103"]
}
```

Every write tool passes the **MCP safety gate** before anything reaches Redis.

## The MCP safety gate

**AI features off.** Before anything else: when the user
turns AI features off in *Ranch › Settings › AI features*, `ranch-mcp`
refuses **every** tool, reads included, with kind `aiDisabled` ("AI features
are turned off in Ranch"). Requests that were waiting for an approval are
denied with that reason. MCP client configurations are left as they are, so
turning AI back on needs no reinstall. Only a human can turn it back on: the
switch is not, and will never be, an MCP tool.

**Job data shared with agents** — *Settings › AI features*
(kv `ai_share_mode`) decides what of a job's data the answers of `list_jobs`,
`inspect_job`, `list_failed_jobs`, `get_flow`, `list_schedulers` and
`get_job_logs` carry:

- **Everything** — payloads (`data`), return values, progress and scheduler
  templates as they are;
- **Mask fields whose name matches** (the default) — the value of every field
  whose name matches a pattern is `"•••"`, at any depth (default patterns
  `email, phone, token, password, secret, iban, card*`; a word matches any
  name that contains it, `*` is a wildcard);
- **Names, states and errors only** — `data`, `returnValue`, `progress` and
  `template` are `null`, job logs are empty, and the answer carries a
  `dataWithheld` sentence saying so.

Failure reasons and stacktraces are errors and are always sent. The policy is
applied by the server to the answer text, so no tool can skip it.

With AI features on, reads are free. Writes are checked in this order:

1. **Read-only for agents** — each connection has an
   `mcpReadonly` setting: `true` (agents can only read), `false` (agents may
   write) or `null` (follow the global *Read-only by default* switch, off
   unless you turn it on). Connections in a Production-colored environment
   start at `true`, and entering Production — by move or by recoloring the
   environment — sets it to `true` again. A blocked call fails with kind
   `blockedReadonly`; the message tells the agent that a human has to allow
   agent writes for the connection, in *Agents › Setup & safety*. Agents can
   see the flag up-front in `list_connections`.
2. **Human approval** — a global mode:
   `writes_only` (default: every write tool waits), `all` (reads that target a
   connection wait too; `list_connections` / `list_environments` stay free) or
   `off`. The call is **held**: Ranch shows the request (agent client, group /
   environment / connection, tool, a preview of the target) and the human
   approves or denies. The agent gets one of:
   - `approvalDenied` — with the human's reason when one was given;
   - `approvalTimeout` — nobody answered within the timeout (default 120 s,
     10–3600 configurable);
   - `hostUnavailable` — the Ranch app is not running (or closed while the
     request was waiting). Checked *before* queuing, so this fails immediately
     rather than after the timeout. Ranch writes a heartbeat every 5 s; older
     than 15 s counts as not running.
   A preview that cannot be built (job not found, Redis unreachable) returns
   that error directly — the human is never asked about something that cannot
   work.
3. **Session lock** — only reached when no approval was needed
   (mode `off`): a locked connection rejects the call with `connectionLocked`.
   **An approved call runs even on a locked connection**: the approval is the
   explicit human gesture, valid for that single operation, recorded in the
   audit log with its `approvalId`.

### How an approval is enforced

`ranch-mcp` and the Ranch app are two processes sharing one SQLite database.
The request is a row in `mcp_pending_approvals`; every state change is a
conditional `UPDATE … WHERE status = <expected>`:

```
ranch-mcp  INSERT  pending
Ranch app  UPDATE  pending  → approved | denied      (decide_pending_approval, UI-only)
ranch-mcp  UPDATE  pending  → expired                (timeout / app gone)
actor      UPDATE  approved → consumed               (same connection + same tool, once)
```

The connection actor — the single enforcement point — only executes if *it*
flips `approved → consumed`. A forged id, a pending / denied / expired request,
a replay, or an approval issued for another connection or another tool is
`approvalInvalid`, and never falls back to "then check the lock instead".
Requests older than one hour are purged.

Errors carry a machine-readable discriminant in `error.data.kind`
(`blockedReadonly`, `connectionLocked`, `connectionNotFound`, …) next to the
human-readable `message`, so an agent can branch without parsing prose.

**Safety controls are never MCP tools.** `list_pending_approvals`,
`decide_pending_approval`, `set_connection_mcp_readonly`,
`set_mcp_safety_settings`, `set_kv_setting`, `unlock_connection`,
`lock_connection`, `remove_connection` and `delete_group` are on a deny-list;
`tests/mcp_safety.rs` fails if one of them — or any tool without a read / write
classification in `commands::registry::MCP_TOOLS` — ever shows up on the live
tool router.

### Lock semantics

- **Locked connections reject all destructive MCP calls** with `connectionLocked`
  + the unlock keystroke hint. This is enforced inside the connection actor
  (the same code path the UI uses), not in the MCP server layer — so MCP can
  never bypass the safety model the human relies on.
- **The lock toggle is UI-only by design.** `unlock_connection` /
  `lock_connection` are explicitly NOT MCP tools; an agent must NOT be able
  to escalate its own privileges. The user must unlock in the Ranch desktop
  UI before destructive agent actions can run.
- **Sentinel test**: `tests/safety_lock.rs` covers every destructive MCP
  tool against a `Locked` connection — adding a new destructive MCP tool
  without lock-check coverage fails CI immediately.

## Activity log

`ranch-mcp` records **every** tool call — reads included — in
`mcp_activity_events`: session, client (from the MCP `initialize` handshake),
tool, read / write, connection, arguments, outcome (`success`, `error`,
`blocked_readonly`, `denied`, `timeout`, `host_unavailable`), duration, and the
approval the call went through. It is visible in *Agents › Activity*
(events, sessions, filters, JSON / CSV export, clear, on / off).

- Recording happens in the server's `call_tool`, so no tool can forget it, and
  is best-effort: a logging failure never fails the agent's call.
- A session is one `ranch-mcp` process; it rotates after 10 minutes of silence
  (`mcp_session_gap_minutes`).
- Capped at 5,000 events (`mcp_activity_max_entries`), oldest first out. Long
  id lists are stored as `{ count, first: [...] }`.
- It is **not** the audit log: destructive operations from both surfaces stay
  in `audit_log_entries`, append-only and never clearable. Agents can neither
  read nor clear the activity log (deny-list).
- CSV cells are quoted and spreadsheet formulas defused — the text in there is
  agent- and Redis-controlled.

## Prompt-injection isolation

**Job payloads are opaque JSON.** Every field returned by `inspect_job` /
`list_jobs` (`name`, `data`, `failedReason`, `stacktrace`, `returnvalue`, log
lines) is passed through as-is. The MCP server **never** interprets any field
as an instruction or tool invocation. A canary integration test asserts that
a job whose `name` contains `<system>ignore previous instructions</system>`
round-trips as the literal string with no side effect.

## Architecture: UI ⊇ MCP parity contract

Both surfaces (the Tauri UI's invoke handler and the rmcp tool router) call
the **same** Rust `*_inner` function for each operation. The parity test
(`src-tauri/tests/parity_contract.rs`) enforces: every MCP tool must have a
matching UI command. Adding a tool to one side without the other fails CI.
