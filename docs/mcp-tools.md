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
server, with a fixed argument vector, never a shell string. The CLI is looked
up on `PATH` and in the usual install locations (a macOS GUI app does not
inherit your shell's `PATH`); when it is not found the card says so and shows
the exact command to run yourself.

**By hand**, for any other MCP client: add this to its config, e.g.
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

### Read (13), always allowed

| Tool | Purpose |
|---|---|
| `list_connections` | Every saved connection, each with its `groupId` / `groupName`, `environmentId` / `envLabel` / `envColor`, `backend` (the kind of queue system: `bullmq-redis`), `endpoint` (where it points, in one line and without credentials: `redis://host:6379/0`, `rediss://` with TLS, "via SSH" through a tunnel) and `mcpReadonly`. |
| `list_environments` | Every environment (e.g. local, staging, production) in sidebar order, each with its `groupId` / `groupName`, typically one group per project, one environment per stage. |
| `list_queues` | BullMQ queues on a connection, each with its `id` (what the other tools take as `queueId`), its `name`, `isPaused` and `counts`: jobs per state code (`{"waiting": 12, "failed": 3, …}`); a state that is absent holds no job. |
| `list_jobs` | Paginated job list per `(queue, state)`; `state` is one of the codes its schema lists (`waiting`, `active`, `completed`, `failed`, `delayed`, `paused`, and BullMQ's `prioritized`, `waitingChildren`, `stalled`). Optional `search` (job id or name), `reason` (failure reason), `sortBy` (`name` \| `queuedAt` \| `duration`) + `sortDescending` run across the whole state (the newest 10,000 jobs), not just the returned page; `total` is then the number of matching jobs and `scanCapped` says whether older jobs were left out. Jobs carry `progress` (raw JSON of `job.updateProgress()`). Rows are **summaries** (see below). |
| `inspect_job` | Everything about one job: payload, options, return value, progress, the whole `failedReason` and its stacktraces. BullMQ keeps one trace per attempt; identical ones are returned once, and `stacktraceIdenticalAttempts` says how many attempts produced it. |
| `list_failed_jobs` | **Everything that failed on a connection, across all its queues**, newest first: the jobs (paged, `limit` ≤ 250), per-queue counts, and the most failing job types. Optional filters `queue` (a queue's name), `name` (a job name, exact), `reason` (substring). Looks at the newest 500 failed jobs per queue: "what failed on staging?" is one call. |
| `get_job_logs` | A page of a job's BullMQ log lines (`offset` default 0, `limit` default 100, capped at 500). |
| `list_schedulers` | Schedulers (CRON / repeatable jobs) of a queue: key, pattern, next run, paused. |
| `get_flow` | The flow (BullMQ `FlowProducer` parent / child tree) a job belongs to, as nested JSON from the root (per job: queue, id, name, state, duration, failure reason, children), across queues. "Why is this parent stuck in `waitingChildren`?" = the children that are not `completed`. A job in no flow returns a tree of one; cut at 200 jobs (`truncated`). |
| `list_workers` | BullMQ workers attached to a queue (id, name, address, `ageSecs`, `idleSecs`, last command), read from `CLIENT LIST`. `workers: []` = nothing is consuming the queue. `supported: false` = this Redis provider forbids `CLIENT LIST` (managed Redis, ACL), which says nothing about the workers. |
| `get_queue_metrics` | Metrics of one queue over a window (`range`: `5m`, `15m`, `30m`, `1h`, `6h`, `24h` default, `3d`, `7d`): counts and per-bucket series, throughput, failure rate, processing time and queue delay (avg / min / max), the 10 slowest jobs and the 10 most failing job types. `nativeQueues` / `totalQueues` tell whether the counts are exact (the queue's Worker records BullMQ metrics) or estimated from the jobs still in Redis. |
| `get_connection_metrics` | The same, merged over every queue of a connection. |
| `list_failure_causes` | The failures of a connection (optional `queue`, by name) **grouped by cause** (the reason with its values replaced, as the Failed tab shows them), most frequent first: per cause its `signature`, a sample reason, the count, first and last time, job names, queues, up to 20 of its jobs, and `noted` when a note already explains it. The signature is the key for `annotate_cause`. |

With approval mode `all`, the eleven read tools that target a connection are
held like writes; `list_connections` and `list_environments` always stay free.

#### A list is for choosing, `inspect_job` is for reading

A row of `list_jobs` / `list_failed_jobs` carries what it takes to pick a job:
its `queueId` + `jobId` (exactly what the other tools take, so a row can be
handed straight to `inspect_job` or `retry_job`), its `queue` name, state, timing, the
payload, and a `failedReason` capped at 800 characters
(`failedReasonTruncated: true` when it was cut). It carries **no stacktrace and
no `opts`**: `stacktraceEntries` says how many traces the job holds, and
`inspect_job` gives them.

This is a deliberate budget. Stacktraces make up most of a failed job, and
BullMQ stores one per attempt, so a job that failed three times on one bug
holds the same trace three times. Five failed jobs as summaries take about
5 KB; with every trace they would take about 35 KB.

Paging is optional everywhere it has an obvious default: `offset` is 0, and
`limit` is 50 (at most 100) for `list_jobs`, 50 (at most 250) for
`list_failed_jobs` and 100 (at most 500) for `get_job_logs`; `list_jobs` takes
`newestFirst` (true). Only what cannot be guessed (the connection, the queue,
the state) is required. `queueId` and `state` are described **inline** in
every tool's schema rather than `$ref`'d, because some clients hand the agent
a `$ref`'d subschema as `{}`.

**A queue is named by its id.** `list_queues` gives each queue an `id` and a
`name`; every other tool takes the `queueId` and every job a tool returns
carries it. The id is opaque: pass it as given, never build one. On BullMQ it
reads `{prefix}:{name}` (`bull:emails`), the base of the queue's Redis keys.

`data`, `opts`, `returnValue` and `progress` are returned as **JSON values**,
not as strings containing JSON: BullMQ stores them encoded, and an agent should
not pay for the escaping and then parse it itself. A field that does not parse
is passed through as the string it is, never dropped.

### Write (16), behind the safety gate

| Tool | Purpose |
|---|---|
| `retry_job` | Move a failed job back to `waiting`. |
| `remove_job` | Delete a job from every state list + hash + logs. |
| `promote_job` | Move a delayed job into `waiting` immediately. |
| `bulk_retry_jobs` / `bulk_remove_jobs` / `bulk_promote_jobs` | The same for up to 1,000 job ids of one queue, as **one operation and one approval**. Returns per-job outcomes. `bulk_retry_jobs` also takes `allFailed: true` (instead of `jobIds`): every failed job of the queue, however many: the approval states the count. |
| `drain_queue` | Remove every job that has **not started**: waiting, paused, prioritized and, unless `includeDelayed: false`, delayed. Active, completed and failed jobs stay. The pending run of a job scheduler and members of a flow are left in place (`skipped`). Cannot be undone. |
| `clean_queue` | Remove the `completed` or `failed` jobs (`state`) that finished more than `olderThanMs` ago (`0` = all). Members of a flow are left in place. Cannot be undone. |
| `pause_queue` / `resume_queue` | Stop / restart job pickup on a queue. Idempotent. |
| `pause_scheduler` / `resume_scheduler` | Stop / restart one scheduler (`schedulerKey` from `list_schedulers`). Pausing removes its pending job (nothing fires) and keeps it listed, flagged `paused`; resuming re-arms it from its stored settings. |
| `upsert_scheduler` | Create a job scheduler, or change an existing one (same `schedulerId`): `jobName` + either a cron `pattern` (5 fields, or 6 with seconds first; numbers, `*`, lists, ranges, steps, month / weekday names; optional IANA `tz`) **or** `everyMs` (≥ 1000), and an optional JSON `data` template. Returns `nextRunMs` and the pending job's id. The approval shows the schedule, the next run and the template; an invalid pattern or timezone is refused before anyone is asked. |
| `remove_scheduler` | Delete a job scheduler and its pending job. A job of it that is running finishes; nothing further is produced. |
| `annotate_cause` | Attach a **resolution note** to a failure cause: `queueId` (of one of the cause's jobs) and, optionally, its `queue` name to show with the note, `signature` (from `list_failure_causes`), `text` (what is wrong, ≤ 4,000 characters), `steps` (what to do, ≤ 20), `jobsRead`. Shown next to the cause in Ranch and under Agents › Notes, with the agent client and the time. **No approval**: a note changes no queue state; it is refused on a connection read-only for agents and, like every tool, when AI features are off. |
| `add_job` | Enqueue a job on an **existing** queue: `name`, `data` (any JSON), optional `delayMs`, `attempts`, `priority` (1 = highest), `lifo`. Returns `jobId` and the `state` it landed in (`waiting`, `paused`, `delayed`, `prioritized`). The approval preview shows the queue, the name, the options and the whole payload. An invalid job or an unknown queue is refused before anyone is asked. |

**One call = one approval.** That is why the bulk tools exist on this surface:
"remove these 12 jobs" is one decision for the human, with the count and the
first job names in the approval preview, not twelve modals.

Example (`bulk_remove_jobs`):

```json
{
  "connectionId": "0198…",
  "queueId": "bull:report-generation",
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

**Job data shared with agents.** *Settings › AI features*
decides what of a job's data the answers of `list_jobs`,
`inspect_job`, `list_failed_jobs`, `get_flow`, `list_schedulers` and
`get_job_logs` carry:

- **Everything**: payloads (`data`), return values, progress and scheduler
  templates as they are;
- **Mask fields whose name matches** (the default): the value of every field
  whose name matches a pattern is `"•••"`, at any depth (default patterns
  `email, phone, token, password, passwd, secret, credential, apikey,
  api_key, authorization, cookie, iban, card*`; a word matches any name that
  contains it, `*` is a wildcard). Job log lines are free text with no field
  names: this mode sends them as they are;
- **Names, states and errors only**: `data`, `returnValue`, `progress` and
  `template` are `null`, job logs are empty, and the answer carries a
  `dataWithheld` sentence saying so.

Failure reasons and stacktraces are errors and are always sent: a worker that
puts a secret in an error message shares it with the agent in every mode. The policy is
applied by the server to the answer text, so no tool can skip it.

With AI features on, reads are free. Writes are checked in this order:

1. **Read-only for agents.** Each connection has an
   `mcpReadonly` setting: `true` (agents can only read), `false` (agents may
   write) or `null` (follow the global *Read-only by default* switch, off
   unless you turn it on). Connections in a Production-colored environment
   start at `true`, and entering Production (by move or by recoloring the
   environment) sets it to `true` again. A blocked call fails with kind
   `blockedReadonly`; the message tells the agent that a human has to allow
   agent writes for the connection, in *Agents › Setup & safety*. Agents can
   see the flag up-front in `list_connections`.
2. **Human approval**, a global mode:
   `writes_only` (default: every write tool waits), `all` (reads that target a
   connection wait too; `list_connections` / `list_environments` stay free) or
   `off`. The call is **held**: Ranch shows the request (agent client, group /
   environment / connection, tool, a preview of the target) and the human
   approves or denies. The agent gets one of:
   - `approvalDenied`, with the human's reason when one was given;
   - `approvalTimeout`: nobody answered within the timeout (default 120 s,
     10–3600 configurable);
   - `hostUnavailable`: the Ranch app is not running (or closed while the
     request was waiting). Checked *before* queuing, so this fails immediately
     rather than after the timeout. Ranch writes a heartbeat every 5 s; older
     than 15 s counts as not running.
   A preview that cannot be built (job not found, Redis unreachable) returns
   that error directly: the human is never asked about something that cannot
   work.
3. **Session lock**, only reached when no approval was needed
   (mode `off`): a locked connection rejects the call with `connectionLocked`.
   **An approved call runs even on a locked connection**: the approval is the
   explicit human gesture, valid for that single operation, recorded in the
   audit log with its `approvalId`.

### How an approval is enforced

An approval is valid once, for the connection, the tool and the arguments the
human was shown: Ranch runs the call only when its own seal over that decision
matches the call `ranch-mcp` holds, and spends the approval as it runs. A
forged id, a pending, denied or expired request, a replay, or an approval for
another connection or tool fails with `approvalInvalid`, and never falls back
to "then check the lock instead". Undecided requests older than one hour are
purged.

The safety settings and each connection's read-only flag and lock are sealed
the same way, and a value without the app's seal counts as its strictest
setting. A process that edits Ranch's database (an agent with a shell, say)
approves nothing and opens nothing (on Linux, where the Secret Service gives
the key to any process of the user, this holds only as far as the keychain
does).

**Safety controls are never MCP tools.** Approving or listing requests,
the read-only flags and safety settings, the generic settings writer,
locking and unlocking, removing a connection or a group, the activity log,
the client installers and anything that starts a process are left to the
human, in the Ranch app.

### Lock semantics

- **Locked connections reject every write tool that changes queues** with
  `connectionLocked` and the unlock keystroke hint. The check is the one the
  UI's own writes go through, so an agent cannot bypass the lock the human
  relies on.
- **The lock toggle is UI-only by design.** An agent cannot unlock a
  connection; the user unlocks it in the Ranch app before agent writes can run
  without an approval.

## Errors

A tool error carries a machine-readable `error.data.kind` next to the
human-readable `message`, so an agent can branch without parsing prose:

| Kind | Meaning | What to do |
|---|---|---|
| `aiDisabled` | The user turned AI features off in Ranch. | Stop calling tools and tell the user. |
| `blockedReadonly` | The connection is read-only for agents (`mcpReadonly` in `list_connections`). | Tell the user; only they can allow writes, in *Agents › Setup & safety*. |
| `approvalDenied` | The user said no, with their reason when given. | Respect it; do not retry. |
| `approvalTimeout` | Nobody answered in time. | Ask the user before trying again. |
| `approvalInvalid` | The approval does not authorize this call. | Do not retry the same call without a new approval. |
| `hostUnavailable` | The Ranch app is not running. | Ask the user to open Ranch. |
| `connectionLocked` | The connection is locked and no approval was needed (approval mode `off`). | Ask the user to unlock it in Ranch. |
| `unsupported` | The connection's backend does not provide this, named in the message. | Use another tool or tell the user. |
| `connectionNotFound`, `jobNotFound`, `schedulerNotFound` | The id does not name anything (anymore). | List again. |
| `invalidJobState` | The job is not in a state the action applies to (retrying a job that has not failed). | Read the job again. |
| `invalidInput` | An argument was refused, with the reason (an invalid cron pattern, an unknown state code). | Fix the argument. |
| `redis`, `sshAuthFailed`, `sshTunnelFailed`, `schemaNotRecognized` | Ranch could not reach or read the connection's server. | Tell the user; the message says why. |

## Activity log

`ranch-mcp` records **every** tool call, reads included, with its session,
client (from the MCP `initialize` handshake), tool, connection, arguments,
outcome, duration and the approval the call went through. The user sees it in
*Agents › Activity*.

- Recording cannot be skipped by a tool, and is best effort: a logging failure
  never fails the call.
- A session is one `ranch-mcp` process; it rotates after 10 minutes of silence.
- It keeps the newest 5,000 events. Long id lists are stored as
  `{ count, first: [...] }`.
- It is **not** the audit log: every write from the UI or an agent also lands
  in the audit log, which is append-only. Agents can neither read nor clear
  the activity log.

## Prompt-injection isolation

**Job payloads are opaque JSON.** Every field returned by `inspect_job` /
`list_jobs` (`name`, `data`, `failedReason`, `stacktrace`, `returnvalue`, log
lines) is passed through as-is. The MCP server **never** interprets any field
as an instruction or tool invocation: a job named
`<system>ignore previous instructions</system>` comes back as that literal
string, with no side effect.
