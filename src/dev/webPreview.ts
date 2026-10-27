/**
 * **Ranch web** — the app in a plain browser, for UI work and review.
 *
 * `pnpm dev` + http://localhost:1420 outside Tauri has no backend: every
 * `invoke` throws and the app cannot even mount. This module stands in for
 * the backend with a small, **stateful, in-memory BullMQ world** — groups,
 * connections (one unreachable, one locked), queues with jobs in every state,
 * schedulers, a flow, workers, audit entries, agent activity and a pending
 * agent approval — so that every screen, dialog and empty / error state can
 * be looked at, clicked through and screenshotted.
 *
 * Loaded by `main.tsx` only when `import.meta.env.DEV` **and** there is no
 * Tauri runtime; it is a dynamic import, so it is not in production bundles.
 * Writes mutate the in-memory world (retry moves the job, drain empties the
 * lists…); a reload starts over. Nothing here talks to Redis.
 *
 * `?preview=empty` shows a first launch (no connection, the wizard);
 * `?preview=full` brings the demo world back.
 *
 * Helpers on `window.ranchPreview`: `requestApproval(tool?)` queues another
 * agent approval, `reset()` rebuilds the world.
 */
import { installTauriShim } from "@/lib/runtime/tauriShim"
import type {
  ActivityEvent,
  AuditLogEntry,
  Connection,
  CronJob,
  Environment,
  FlowNode,
  Group,
  Job,
  JobState,
  McpInstallerStatus,
  McpSafetySettings,
  PendingApproval,
  Queue,
  QueueMetrics,
  QueueRef,
  QueueWorker,
} from "@/lib/tauri/bindings"

type Args = Record<string, unknown>
type Handler = (args: Args) => unknown

const MIN = 60_000
const HOUR = 60 * MIN
const DAY = 24 * HOUR

class PreviewError {
  constructor(
    public kind: string,
    public message: string,
    public docUrl: string | null = null,
  ) {}
}

// ── deterministic pseudo-random, so screenshots are stable ────────────────
let seed = 42
function rnd(): number {
  seed = (seed * 1664525 + 1013904223) % 4294967296
  return seed / 4294967296
}
const pick = <T>(list: readonly T[]): T => list[Math.floor(rnd() * list.length)] as T
const between = (min: number, max: number) => Math.floor(min + rnd() * (max - min))

// ── the world ────────────────────────────────────────────────────────────
interface QueueWorld {
  ref: QueueRef
  paused: boolean
  jobs: Map<JobState, Job[]>
  schedulers: CronJob[]
  workers: QueueWorker[]
  nextId: number
}

interface World {
  groups: Group[]
  environments: Environment[]
  connections: Connection[]
  unlocked: Set<string>
  queues: Map<string, QueueWorld[]>
  kv: Map<string, string>
  safety: McpSafetySettings
  audit: AuditLogEntry[]
  activity: ActivityEvent[]
  approvals: PendingApproval[]
  installers: Record<string, McpInstallerStatus>
}

const FAILURES = [
  "connect ETIMEDOUT 10.0.4.12:5432",
  "Request failed with status code 503",
  'ValidationError: "email" must be a valid email',
  "RateLimitError: 429 Too Many Requests — retry after 30s",
  "TypeError: Cannot read properties of undefined (reading 'id')",
]

function makeJob(queue: QueueRef, id: string, name: string, state: JobState, now: number): Job {
  const queuedAgo = between(2 * MIN, 3 * DAY)
  const timestamp = now - queuedAgo
  const started = timestamp + between(200, 40_000)
  const took = between(40, 9_000)
  const finished = state === "completed" || state === "failed"
  const failed = state === "failed"
  // One delayed job in three is between two attempts: it failed once and
  // waits for its backoff — BullMQ keeps the reason and the trace on it.
  const retrying = state === "delayed" && between(0, 2) === 0
  const reason = failed || retrying ? pick(FAILURES) : null
  // BullMQ stores one whole `err.stack` per failed attempt.
  const trace = (n: number) =>
    `Error: ${reason}\n    at sendEmail (/app/dist/workers/email.js:${41 + n}:13)\n    at async Worker.processFn (/app/dist/workers/index.js:18:9)\n    at async Worker.callProcessJob (/app/node_modules/bullmq/dist/cjs/classes/worker.js:178:21)\n    at async Worker.retryIfFailed (/app/node_modules/bullmq/dist/cjs/classes/worker.js:581:24)\n    at process.processTicksAndRejections (node:internal/process/task_queues:95:5)`
  return {
    jobRef: { queue, id },
    name,
    data: JSON.stringify({
      userId: `u_${between(1000, 9999)}`,
      to: `user${between(1, 500)}@example.com`,
      template: pick(["welcome", "invoice", "reset-password", "digest"]),
      locale: pick(["fr-BE", "nl-BE", "en-GB"]),
    }),
    opts: JSON.stringify({ attempts: 3, backoff: { type: "exponential", delay: 1000 } }),
    state,
    timestamp,
    processedOn: finished || retrying || state === "active" ? started : null,
    finishedOn: finished ? started + took : null,
    // BullMQ counts an attempt when it ends: a first run in progress is 0.
    attemptsMade: failed ? 3 : finished || retrying ? 1 : 0,
    delay: retrying ? 2_000 : state === "delayed" ? between(1, 60) * MIN : null,
    failedReason: reason,
    stacktrace: reason ? Array.from({ length: failed ? 3 : 1 }, (_, n) => trace(n)) : [],
    returnValue: state === "completed" ? JSON.stringify({ delivered: true, ms: took }) : null,
    parentKey: null,
    progress:
      state === "active"
        ? pick([
            "42",
            "87",
            JSON.stringify({ step: "upload", done: 3, of: 8 }),
            JSON.stringify({ percentage: 64, message: "page 16 of 25" }),
          ])
        : null,
    // Who runs it, and its lock while it runs. Derived from the
    // id, not drawn: drawing would shift the rest of the demo world.
    processedBy:
      finished || retrying || state === "active"
        ? `worker-${queue.name}-${(Number(id) % 3) + 1}`
        : null,
    lockTtlMs: state === "active" ? 4_000 + ((Number(id) * 7_919) % 25_000) : null,
  }
}

function makeQueue(
  prefix: string,
  name: string,
  names: string[],
  counts: Partial<Record<JobState, number>>,
  now: number,
): QueueWorld {
  const ref = { prefix, name }
  const jobs = new Map<JobState, Job[]>()
  let nextId = 1
  for (const state of [
    "completed",
    "failed",
    "delayed",
    "prioritized",
    "waiting",
    "active",
    "paused",
  ] as JobState[]) {
    const list: Job[] = []
    for (let i = 0; i < (counts[state] ?? 0); i++) {
      list.push(makeJob(ref, String(nextId++), pick(names), state, now))
    }
    list.sort((a, b) => (b.finishedOn ?? b.timestamp) - (a.finishedOn ?? a.timestamp))
    jobs.set(state, list)
  }
  return { ref, paused: false, jobs, schedulers: [], workers: [], nextId }
}

/**
 * A queue shaped like a real one that broke the first table design: one job
 * name for every job, long custom ids built from a uuid that is also in the
 * payload, and multi-line Prisma failures whose message is on the last line.
 */
function enrichmentQueue(now: number): QueueWorld {
  const queue = makeQueue("bull", "enrichment", ["enrich-film"], { completed: 60, failed: 2 }, now)
  const hex = (n: number) =>
    Array.from({ length: n }, () => "0123456789abcdef"[between(0, 15)]).join("")
  const tables = ["movie_credits", "movie_production_countries"]
  for (const list of queue.jobs.values()) {
    list.forEach((job, i) => {
      const uuid = `${hex(8)}-${hex(4)}-4${hex(3)}-a${hex(3)}-${hex(12)}`
      job.jobRef = { ...job.jobRef, id: `enrich-film-${uuid}` }
      job.data = JSON.stringify({
        filmId: uuid,
        source: "TMDB",
        force: i % 5 === 0,
        locale: "fr-BE",
      })
      if (job.failedReason) {
        const table = tables[i % tables.length]
        job.failedReason = [
          "",
          `Invalid \`tx.${table}.createMany()\` invocation in`,
          "/app/apps/api/src/catalog/film/infrastructure/persistence/film.prisma-repository.ts:416:45",
          "",
          `  413 await tx.${table}.deleteMany({ where: { movie_id: film.id } });`,
          "  414 ",
          "  415 if (rows.length > 0) {",
          `→ 416   await tx.${table}.createMany(`,
          "Unique constraint failed on the fields: (`movie_id`,`country_code`)",
        ].join("\n")
        job.stacktrace = job.stacktrace.map(
          () =>
            `PrismaClientKnownRequestError: ${job.failedReason}\n    at $n.handleRequestError (/app/node_modules/@prisma/client/runtime/library.js:121:7315)\n    at FilmPrismaRepository.save (/app/apps/api/dist/film.prisma-repository.js:416:45)`,
        )
      }
    })
  }
  return queue
}

function buildWorld(): World {
  seed = 42
  const now = Date.now()
  const groups: Group[] = [
    {
      id: "g-lumen",
      name: "Lumen Studios",
      displayOrder: 0,
      collapsed: false,
      connectionCount: 3,
    },
    { id: "g-side", name: "Side project", displayOrder: 1, collapsed: false, connectionCount: 1 },
    {
      id: "g-empty",
      name: "Client — Atelier Nord",
      displayOrder: 2,
      collapsed: false,
      connectionCount: 0,
    },
  ]
  const env = (id: string, groupId: string, name: string, label: string, colorToken: string) => ({
    id,
    groupId,
    groupName: groups.find((g) => g.id === groupId)?.name ?? "",
    name,
    label,
    colorToken,
  })
  const environments: Environment[] = [
    env("e-t-local", "g-lumen", "local", "Local", "env-local"),
    env("e-t-staging", "g-lumen", "staging", "Staging", "env-staging"),
    env("e-t-prod", "g-lumen", "production", "Production", "env-production"),
    env("e-s-local", "g-side", "local", "Local", "env-local"),
  ]
  const conn = (
    id: string,
    environmentId: string,
    name: string,
    host: string,
    over: Partial<Connection> = {},
  ): Connection => {
    const e = environments.find((x) => x.id === environmentId) as Environment
    return {
      id,
      environmentId,
      groupId: e.groupId,
      name,
      host,
      port: 6379,
      db: 0,
      username: null,
      tls: false,
      tlsCustomCa: null,
      sshTunnel: null,
      envColor: e.colorToken,
      envName: e.name,
      envLabel: e.label,
      groupName: e.groupName,
      createdAt: new Date(now - 40 * DAY).toISOString(),
      mcpReadonly: null,
      sidebarCollapsed: false,
      ...over,
    }
  }
  const connections: Connection[] = [
    conn("c-local", "e-t-local", "lumen-api", "127.0.0.1", { port: 6390 }),
    conn("c-staging", "e-t-staging", "lumen-api", "redis.staging.lumen.internal", { tls: true }),
    conn("c-prod", "e-t-prod", "lumen-api", "redis.prod.lumen.internal", {
      tls: true,
      username: "ranch",
      mcpReadonly: true,
      sshTunnel: {
        host: "bastion.lumen.example",
        port: 22,
        user: "deploy",
        auth: { kind: "keyFile", keyFilePath: "~/.ssh/id_ed25519" },
        jumpHost: null,
      },
    }),
    conn("c-side", "e-s-local", "playground", "localhost", { port: 6380, db: 2 }),
  ]

  const emailNames = ["send-email", "send-reminder", "send-invoice", "send-digest"]
  const queuesFor = (scale: number): QueueWorld[] => {
    const emails = makeQueue(
      "bull",
      "emails",
      emailNames,
      {
        completed: 180 * scale,
        failed: 37 * scale,
        waiting: 64 * scale,
        active: 3,
        delayed: 12,
        prioritized: 4,
      },
      now,
    )
    emails.schedulers = [
      {
        key: "daily-report",
        name: "send-daily-report",
        pattern: "0 9 * * *",
        every: null,
        tz: "Europe/Brussels",
        offset: 0,
        iterationCount: 128,
        nextRun: now + 5 * HOUR,
        endDate: null,
        template: JSON.stringify({ reportType: "summary" }),
        paused: false,
      },
      {
        key: "health-ping",
        name: "ping-health",
        pattern: null,
        every: 15 * MIN,
        tz: null,
        offset: 228_813,
        iterationCount: 9_412,
        nextRun: now + 4 * MIN,
        endDate: null,
        template: null,
        paused: true,
      },
    ]
    emails.workers = [
      {
        id: "112",
        name: "mailer-eu-1",
        addr: "10.0.3.17:51234",
        ageSecs: 86_400 * 3 + 120,
        idleSecs: 1,
        lastCommand: "bzpopmin",
        library: "ioredis 5.4.1",
        raw: {
          id: "112",
          addr: "10.0.3.17:51234",
          name: "bull:ZW1haWxz:w:mailer-eu-1",
          age: "259320",
          idle: "1",
          flags: "b",
          db: "0",
          cmd: "bzpopmin",
          user: "default",
        },
      },
      {
        id: "113",
        name: null,
        addr: "10.0.3.18:40110",
        ageSecs: 5_400,
        idleSecs: 640,
        lastCommand: "evalsha",
        library: "ioredis 5.4.1",
        raw: {
          id: "113",
          addr: "10.0.3.18:40110",
          name: "bull:ZW1haWxz",
          age: "5400",
          idle: "640",
          flags: "N",
          db: "0",
          cmd: "evalsha",
          user: "default",
        },
      },
    ]
    const images = makeQueue(
      "bull",
      "images",
      ["resize", "thumbnail", "transcode"],
      { completed: 96 * scale, failed: 5, paused: 22, delayed: 2 },
      now,
    )
    images.paused = true
    const reports = makeQueue(
      "bull",
      "reports",
      ["build-report"],
      { completed: 12, waiting: 2 },
      now,
    )
    const orders = makeQueue("bull", "orders", ["assemble-order"], { completed: 8 }, now)
    const idle = makeQueue("bull", "webhooks-outbound-retries-eu-west", ["deliver"], {}, now)
    return [emails, images, reports, orders, idle, enrichmentQueue(now)]
  }
  const queues = new Map<string, QueueWorld[]>([
    ["c-local", queuesFor(1)],
    ["c-staging", queuesFor(3)],
    ["c-prod", queuesFor(6)],
  ])

  // One flow in c-local: orders/assemble-order ← emails + images children.
  const local = queues.get("c-local") as QueueWorld[]
  const orders = local.find((q) => q.ref.name === "orders") as QueueWorld
  const parent = makeJob(orders.ref, "flow-1", "assemble-order", "waitingChildren", now)
  orders.jobs.set("waitingChildren", [parent])
  const emailsQ = local.find((q) => q.ref.name === "emails") as QueueWorld
  const childOk = (emailsQ.jobs.get("completed") as Job[])[0] as Job
  const childBad = (emailsQ.jobs.get("failed") as Job[])[0] as Job
  childOk.parentKey = "bull:orders:flow-1"
  childBad.parentKey = "bull:orders:flow-1"

  const audit: AuditLogEntry[] = []
  const auditRow = (
    ago: number,
    action: AuditLogEntry["action"],
    target: AuditLogEntry["target"],
    over: Partial<AuditLogEntry> = {},
  ) =>
    audit.push({
      id: `a${audit.length + 1}`,
      createdAt: new Date(now - ago).toISOString(),
      source: "ui",
      operatorId: "alex",
      agentClient: null,
      agentTransport: null,
      action,
      target,
      outcome: "success",
      outcomeReason: null,
      connectionId: "c-staging",
      connectionName: "lumen-api",
      envColor: "env-staging",
      groupName: "Acme",
      environmentName: "Staging",
      approvalId: null,
      ...over,
    })
  auditRow(
    3 * MIN,
    "retryAllFailed",
    { kind: "queue", data: { queueName: "emails" } },
    { outcomeReason: "111 retried" },
  )
  auditRow(
    9 * MIN,
    "addJob",
    { kind: "job", data: { queueName: "emails", jobId: "4811" } },
    { outcomeReason: "send-email → waiting" },
  )
  auditRow(
    22 * MIN,
    "removeJob",
    { kind: "bulkJobs", data: { queueName: "emails", jobCount: 12 } },
    {
      source: "agent",
      agentClient: "Claude Desktop",
      agentTransport: "stdio",
      approvalId: "ap-prev-1",
    },
  )
  auditRow(
    48 * MIN,
    "unlockConnection",
    { kind: "connection" },
    { connectionId: "c-prod", connectionName: "lumen-api", envColor: "env-production" },
  )
  auditRow(
    2 * HOUR,
    "upsertScheduler",
    { kind: "scheduler", data: { queueName: "emails", schedulerKey: "daily-report" } },
    { outcomeReason: "send-daily-report · 0 9 * * * (Europe/Brussels)" },
  )
  auditRow(
    3 * HOUR,
    "drainQueue",
    { kind: "queue", data: { queueName: "images" } },
    { outcome: "failure", outcomeReason: "0 removed — stopped on error: Connection reset by peer" },
  )
  auditRow(5 * HOUR, "pauseQueue", { kind: "queue", data: { queueName: "images" } })
  auditRow(
    DAY,
    "retryJob",
    { kind: "job", data: { queueName: "emails", jobId: "3120" } },
    {
      source: "agent",
      agentClient: "Cursor",
      agentTransport: "stdio",
      outcome: "cancelled",
      outcomeReason: "approval denied",
    },
  )

  const activity: ActivityEvent[] = []
  const act = (ago: number, tool: string, over: Partial<ActivityEvent> = {}) =>
    activity.push({
      id: `ev${activity.length + 1}`,
      sessionId: ago < HOUR ? "s-now" : "s-morning",
      timestamp: new Date(now - ago).toISOString(),
      clientHint: ago < HOUR ? "Claude Desktop" : "Cursor",
      tool,
      kind: "read",
      connectionId: "c-staging",
      connectionName: "lumen-api",
      targetJson: JSON.stringify({ queue: "emails" }),
      status: "success",
      durationMs: between(8, 240),
      error: null,
      approvalId: null,
      ...over,
    })
  act(2 * MIN, "list_failed_jobs")
  act(3 * MIN, "inspect_job", { targetJson: JSON.stringify({ queue: "emails", jobId: "217" }) })
  act(4 * MIN, "bulk_retry_jobs", {
    kind: "write",
    status: "success",
    approvalId: "ap-prev-2",
    durationMs: 9_400,
  })
  act(6 * MIN, "remove_job", {
    kind: "write",
    status: "denied",
    error: "Denied by the user: not now",
  })
  act(7 * MIN, "drain_queue", {
    kind: "write",
    status: "blocked_readonly",
    connectionId: "c-prod",
    error: "This connection is read-only for agents",
  })
  act(5 * HOUR, "list_queues")
  act(5 * HOUR + MIN, "add_job", {
    kind: "write",
    status: "timeout",
    error: "Nobody answered within 120 s",
  })
  act(5 * HOUR + 2 * MIN, "get_queue_metrics")

  const installer = (
    client: McpInstallerStatus["client"],
    configPath: string,
    over: Partial<McpInstallerStatus> = {},
  ): McpInstallerStatus => ({
    client,
    configPath,
    installed: false,
    configDirExists: true,
    error: null,
    kind: "jsonMerge",
    cliFound: null,
    manualCommand: null,
    ...over,
  })
  const bin = "/Applications/Ranch.app/Contents/MacOS/ranch-mcp"

  return {
    groups,
    environments,
    connections,
    unlocked: new Set(),
    queues,
    kv: new Map([["selected_connection_id", "c-staging"]]),
    safety: {
      readonlyDefault: false,
      approvalMode: "writes_only",
      approvalTimeoutSeconds: 120,
      approvalAlwaysOnTop: true,
      approvalNotifySound: true,
    },
    audit,
    activity,
    approvals: [],
    installers: {
      claudeDesktop: installer(
        "claudeDesktop",
        "~/Library/Application Support/Claude/claude_desktop_config.json",
        { installed: true },
      ),
      cursor: installer("cursor", "~/.cursor/mcp.json"),
      windsurf: installer("windsurf", "~/.codeium/windsurf/mcp_config.json", {
        configDirExists: false,
      }),
      claudeCode: installer("claudeCode", "~/.claude.json", {
        kind: "clientCli",
        cliFound: true,
        manualCommand: `claude mcp add --scope user ranch ${bin}`,
      }),
      codex: installer("codex", "~/.codex/config.toml", {
        kind: "clientCli",
        cliFound: false,
        manualCommand: `codex mcp add ranch -- ${bin}`,
      }),
    },
  }
}

/**
 * `?preview=empty` opens Ranch as on a first launch — no group, no
 * connection, the first-run wizard — and `?preview=full` brings the demo
 * world back. Remembered for the tab's session, so it survives reloads.
 */
const SCENARIO_KEY = "ranchPreview.scenario"

function readScenario(): "full" | "empty" {
  try {
    const asked = new URLSearchParams(window.location.search).get("preview")
    if (asked === "empty" || asked === "full") sessionStorage.setItem(SCENARIO_KEY, asked)
    return sessionStorage.getItem(SCENARIO_KEY) === "empty" ? "empty" : "full"
  } catch {
    return "full"
  }
}

const previewDismissed = new Set<string>()

function demoNote(conn: Connection, signature: string, dismissed: boolean) {
  return {
    id: "note-1",
    connectionId: conn.id,
    connectionName: conn.name,
    queueRef: { prefix: "bull", name: "emails" },
    signature,
    text: "The SMTP relay rejects the connection since the 14:02 deploy: its new TLS certificate is not trusted by the worker image.",
    steps: [
      "Add the relay's intermediate certificate to the worker image",
      "Redeploy the email workers",
      "Retry the failed jobs of this cause",
    ],
    clientHint: "claude-code",
    createdAt: new Date(Date.now() - 38 * 60_000).toISOString(),
    jobsRead: ["1232", "1231", "1229"],
    dismissedAt: dismissed ? new Date().toISOString() : null,
  }
}

/** `?ai=off` opens the preview with AI features turned off. */
function aiOffAsked(): boolean {
  try {
    return new URLSearchParams(window.location.search).get("ai") === "off"
  } catch {
    return false
  }
}

function buildScenario(): World {
  const world = buildScenarioWorld()
  if (aiOffAsked()) world.kv.set("ai_enabled", "false")
  if (manyQueuesAsked()) addManyQueues(world)
  return world
}

/** `?queues=many`: 42 queues on c-staging, for the Map. */
function manyQueuesAsked(): boolean {
  try {
    return new URLSearchParams(window.location.search).get("queues") === "many"
  } catch {
    return false
  }
}

function addManyQueues(world: World) {
  const list = world.queues.get("c-staging")
  if (!list) return
  const now = Date.now()
  const areas = ["billing", "catalog", "search", "media", "notify", "sync"]
  const kinds = ["import", "export", "index", "resize", "digest", "cleanup"]
  for (let i = 0; list.length < 42; i++) {
    const name = `${areas[i % areas.length]}-${kinds[Math.floor(i / areas.length) % kinds.length]}`
    const busy = i % 3 !== 2
    list.push(
      makeQueue(
        "bull",
        name,
        [`${name}-job`],
        busy ? { completed: 3 + (i % 5), waiting: i % 4 } : {},
        now,
      ),
    )
  }
}

function buildScenarioWorld(): World {
  const full = buildWorld()
  if (readScenario() === "full") return full
  return {
    ...full,
    groups: [],
    environments: [],
    connections: [],
    queues: new Map(),
    kv: new Map(),
    audit: [],
    activity: [],
  }
}

let world = buildScenario()

// ── helpers over the world ───────────────────────────────────────────────
const UNREACHABLE = "c-side"

function connectionOf(id: unknown): Connection {
  const c = world.connections.find((x) => x.id === id)
  if (!c) throw new PreviewError("connectionNotFound", `Connection not found: ${String(id)}`)
  return c
}

function queuesOf(connectionId: unknown): QueueWorld[] {
  connectionOf(connectionId)
  if (connectionId === UNREACHABLE) {
    throw new PreviewError("redis", "Connection refused (os error 61)")
  }
  return world.queues.get(String(connectionId)) ?? []
}

function queueOf(connectionId: unknown, ref: unknown): QueueWorld {
  const name = (ref as QueueRef).name
  const q = queuesOf(connectionId).find((x) => x.ref.name === name)
  if (!q)
    throw new PreviewError("invalidInput", `Queue "${name}" does not exist on this connection`)
  return q
}

function isLocked(connectionId: string): boolean {
  return (
    connectionOf(connectionId).envColor === "env-production" && !world.unlocked.has(connectionId)
  )
}

function guardWrite(connectionId: unknown) {
  const id = String(connectionId)
  if (isLocked(id)) {
    throw new PreviewError(
      "connectionLocked",
      "This production connection is locked. Unlock it for the session (cmd-shift-U) to make changes.",
    )
  }
}

function len(q: QueueWorld, state: JobState): number {
  return q.jobs.get(state)?.length ?? 0
}

function toQueue(q: QueueWorld): Queue {
  return {
    queueRef: q.ref,
    isPaused: q.paused,
    counters: {
      waiting: len(q, "waiting"),
      active: len(q, "active"),
      completed: len(q, "completed"),
      failed: len(q, "failed"),
      delayed: len(q, "delayed"),
      prioritized: len(q, "prioritized"),
      paused: len(q, "paused"),
    },
  } as Queue
}

function findJob(q: QueueWorld, id: string): { job: Job; state: JobState } | null {
  for (const [state, list] of q.jobs) {
    const job = list.find((j) => j.jobRef.id === id)
    if (job) return { job, state }
  }
  return null
}

function moveJob(q: QueueWorld, id: string, from: JobState, to: JobState, patch: Partial<Job>) {
  const list = q.jobs.get(from) ?? []
  const index = list.findIndex((j) => j.jobRef.id === id)
  if (index < 0) return false
  const [job] = list.splice(index, 1) as [Job]
  Object.assign(job, patch, { state: to })
  const target = q.jobs.get(to) ?? []
  target.unshift(job)
  q.jobs.set(to, target)
  return true
}

function audit(
  connectionId: string,
  action: AuditLogEntry["action"],
  target: AuditLogEntry["target"],
  outcomeReason: string | null = null,
) {
  const c = connectionOf(connectionId)
  world.audit.unshift({
    id: `a${world.audit.length + 1}`,
    createdAt: new Date().toISOString(),
    source: "ui",
    operatorId: "alex",
    agentClient: null,
    agentTransport: null,
    action,
    target,
    outcome: "success",
    outcomeReason,
    connectionId,
    connectionName: c.name,
    envColor: c.envColor,
    groupName: c.groupName ?? null,
    environmentName: c.envName ?? null,
    approvalId: null,
  })
}

const RANGE_MS: Record<string, number> = {
  "5m": 5 * MIN,
  "15m": 15 * MIN,
  "30m": 30 * MIN,
  "1h": HOUR,
  "6h": 6 * HOUR,
  "24h": DAY,
  "3d": 3 * DAY,
  "7d": 7 * DAY,
}

function metrics(queues: QueueWorld[], range: string): QueueMetrics {
  const now = Date.now()
  const span = RANGE_MS[range] ?? DAY
  const buckets = 24
  const bucketMs = Math.ceil(span / buckets)
  const start = now - span
  const completedSeries = new Array<number>(buckets).fill(0)
  const failedSeries = new Array<number>(buckets).fill(0)
  const times: number[] = []
  const delays: number[] = []
  const slow: QueueMetrics["slowestJobs"] = []
  // Durations of completed jobs per type, for the medians.
  const byName = new Map<string, { queueName: string; name: string; took: number[] }>()
  const types = new Map<string, QueueMetrics["failingJobTypes"][number]>()
  for (const q of queues) {
    for (const state of ["completed", "failed"] as JobState[]) {
      for (const job of q.jobs.get(state) ?? []) {
        if (!job.finishedOn || job.finishedOn < start) continue
        const b = Math.min(buckets - 1, Math.floor((job.finishedOn - start) / bucketMs))
        if (state === "completed") completedSeries[b] = (completedSeries[b] ?? 0) + 1
        else failedSeries[b] = (failedSeries[b] ?? 0) + 1
        if (job.processedOn) {
          const took = job.finishedOn - job.processedOn
          times.push(took)
          delays.push(job.processedOn - job.timestamp)
          if (state === "completed") {
            const key = `${q.ref.name}/${job.name}`
            const entry = byName.get(key) ?? { queueName: q.ref.name, name: job.name, took: [] }
            entry.took.push(took)
            byName.set(key, entry)
            slow.push({
              queueName: q.ref.name,
              jobId: job.jobRef.id,
              name: job.name,
              processingTimeMs: took,
              finishedOn: job.finishedOn,
            })
          }
        }
        if (state === "failed") {
          const key = `${q.ref.name}/${job.name}`
          const entry = types.get(key) ?? {
            queueName: q.ref.name,
            name: job.name,
            count: 0,
            lastFailedAt: 0,
            lastReason: null,
          }
          entry.count += 1
          if (job.finishedOn > entry.lastFailedAt) {
            entry.lastFailedAt = job.finishedOn
            entry.lastReason = job.failedReason
          }
          types.set(key, entry)
        }
      }
    }
  }
  const sum = (list: number[]) => list.reduce((a, b) => a + b, 0)
  const completedTotal = sum(completedSeries)
  const failedTotal = sum(failedSeries)
  const avg = (list: number[]) => (list.length ? sum(list) / list.length : 0)
  return {
    range: range as QueueMetrics["range"],
    bucketMs,
    bucketStartsMs: completedSeries.map((_, i) => start + i * bucketMs),
    completedTotal,
    failedTotal,
    failureRate: completedTotal + failedTotal ? failedTotal / (completedTotal + failedTotal) : 0,
    throughputPerMinute: completedTotal / (span / MIN),
    completedSeries,
    failedSeries,
    avgProcessingTimeMs: avg(times),
    minProcessingTimeMs: times.length ? Math.min(...times) : 0,
    maxProcessingTimeMs: times.length ? Math.max(...times) : 0,
    avgQueueDelayMs: avg(delays),
    minQueueDelayMs: delays.length ? Math.min(...delays) : 0,
    maxQueueDelayMs: delays.length ? Math.max(...delays) : 0,
    processingTimeSeriesMs: completedSeries.map((n) => (n ? between(300, 4000) : 0)),
    queueDelaySeriesMs: completedSeries.map((n) => (n ? between(500, 30_000) : 0)),
    sampledJobs: times.length,
    usualDurations: [...byName.values()].map(({ queueName, name, took }) => {
      const sorted = [...took].sort((a, b) => a - b)
      const mid = Math.floor(sorted.length / 2)
      const medianMs =
        sorted.length % 2 === 0
          ? Math.round(((sorted[mid - 1] ?? 0) + (sorted[mid] ?? 0)) / 2)
          : (sorted[mid] ?? 0)
      return { queueName, name, medianMs, samples: sorted.length }
    }),
    nativeQueues: queues.length > 1 ? 1 : 0,
    totalQueues: queues.length,
    slowestJobs: slow.sort((a, b) => b.processingTimeMs - a.processingTimeMs).slice(0, 10),
    failingJobTypes: [...types.values()].sort((a, b) => b.count - a.count).slice(0, 10),
  }
}

function nextRuns(schedule: Args): number[] {
  const now = Date.now()
  if (schedule.kind === "every") {
    const every = Number(schedule.everyMs)
    if (!Number.isFinite(every) || every < 1000) {
      throw new PreviewError("invalidInput", "The interval must be at least 1000 ms")
    }
    return [0, 1, 2, 3, 4].map((i) => now + i * every)
  }
  const pattern = String(schedule.pattern ?? "").trim()
  const fields = pattern.split(/\s+/)
  if (fields.length !== 5 && fields.length !== 6) {
    throw new PreviewError(
      "invalidInput",
      `A cron pattern has 5 fields (minute hour day month weekday) or 6 with seconds first — this one has ${fields.length}`,
    )
  }
  if (/[LW#?@]/.test(pattern)) {
    throw new PreviewError(
      "invalidInput",
      "'L' is not supported in a cron pattern here — use numbers, * , - / and month or weekday names",
    )
  }
  // Preview only: not a cron engine — the next five days at the pattern's hour.
  const hour = Number(fields[fields.length === 6 ? 2 : 1])
  const minute = Number(fields[fields.length === 6 ? 1 : 0])
  const base = new Date(now)
  base.setHours(Number.isFinite(hour) ? hour : 9, Number.isFinite(minute) ? minute : 0, 0, 0)
  return [1, 2, 3, 4, 5].map((d) => base.getTime() + d * DAY)
}

function flowFor(connectionId: unknown, ref: QueueRef, jobId: string) {
  const local = queuesOf(connectionId)
  const orders = local.find((q) => q.ref.name === "orders")
  const parent = orders?.jobs.get("waitingChildren")?.[0]
  const emails = local.find((q) => q.ref.name === "emails")
  const children = [
    ...(emails?.jobs.get("completed") ?? []),
    ...(emails?.jobs.get("failed") ?? []),
  ].filter((j) => j.parentKey === "bull:orders:flow-1")
  const focusKey = `${ref.prefix}:${ref.name}:${jobId}`
  const node = (job: Job, parentKey: string | null): FlowNode => ({
    key: `${job.jobRef.queue.prefix}:${job.jobRef.queue.name}:${job.jobRef.id}`,
    jobRef: job.jobRef,
    parentKey,
    name: job.name,
    state: job.state,
    timestamp: job.timestamp,
    durationMs: job.finishedOn && job.processedOn ? job.finishedOn - job.processedOn : null,
    failedReason: job.failedReason,
    missing: false,
    // The demo's failed child has no option letting the parent go on.
    returnValue: job.returnValue,
    blocksParent: parentKey !== null && job.state === "failed",
  })
  const inFlow =
    parent &&
    (focusKey === "bull:orders:flow-1" ||
      children.some((c) => `bull:emails:${c.jobRef.id}` === focusKey))
  if (parent && inFlow) {
    return {
      nodes: [node(parent, null), ...children.map((c) => node(c, "bull:orders:flow-1"))],
      rootKey: "bull:orders:flow-1",
      focusKey,
      truncated: false,
      blocked: children.some((c) => c.state === "failed"),
    }
  }
  const found = findJob(queueOf(connectionId, ref), jobId)
  if (!found) throw new PreviewError("jobNotFound", `Job not found: ${ref.name}/${jobId}`)
  return {
    nodes: [node(found.job, null)],
    rootKey: focusKey,
    focusKey,
    truncated: false,
    blocked: false,
  }
}

function requestApproval(tool = "bulk_remove_jobs") {
  const c = connectionOf("c-staging")
  const previews: Record<string, unknown> = {
    bulk_remove_jobs: {
      kind: "bulk",
      queue: "emails",
      count: 12,
      sampleNames: ["send-email (#217)", "send-invoice (#219)", "send-digest (#230)"],
    },
    add_job: {
      kind: "newJob",
      queue: "emails",
      name: "send-refund-notice",
      options: { attempts: 3, delay: 60000 },
      // One value longer than the dialog on purpose: a payload is whatever
      // the agent sends, and a long unbroken token used to stretch the
      // modal's grid column past the dialog's own width.
      data: {
        orderId: 4711,
        amount: "199.00",
        currency: "EUR",
        receiptUrl:
          "https://billing.internal.example.com/receipts/2026/09/4711-a8f3c2e1-7b6d-4d9a-9f21-c5e0d4b8a913.pdf",
      },
    },
    drain_queue: {
      kind: "maintenance",
      queue: "emails",
      operation: { kind: "drain", includeDelayed: true },
      count: 41200,
      sparedSchedulerJobs: 1,
    },
    upsert_scheduler: {
      kind: "schedulerChange",
      queue: "reports",
      schedulerId: "nightly",
      jobName: "build-report",
      schedule: { kind: "pattern", pattern: "0 3 * * *", tz: "Europe/Brussels" },
      nextRunMs: Date.now() + 7 * HOUR,
      data: { kind: "summary" },
    },
    retry_job: {
      kind: "job",
      queue: "emails",
      jobId: "217",
      name: "send-email",
      state: "failed",
      attemptsMade: 3,
      failedReason: "connect ETIMEDOUT 10.0.4.12:5432",
    },
  }
  world.approvals.push({
    id: `ap-${Date.now()}`,
    createdAt: new Date().toISOString(),
    clientHint: "Claude Desktop",
    connectionId: c.id,
    connectionName: c.name,
    groupName: c.groupName,
    environmentName: c.envLabel,
    envColor: c.envColor,
    tool,
    kind: "write",
    argumentsJson: JSON.stringify({
      connectionId: c.id,
      queueRef: { prefix: "bull", name: "emails" },
    }),
    targetPreviewJson: JSON.stringify(previews[tool] ?? null),
    timeoutSeconds: 120,
  })
}

// ── the command table ────────────────────────────────────────────────────
const a = (args: Args) => (args.args ?? {}) as Args

const commands: Record<string, Handler> = {
  // connections & hierarchy
  list_connections: () => world.connections,
  list_groups: () =>
    world.groups.map((g) => ({
      ...g,
      connectionCount: world.connections.filter((c) => c.groupId === g.id).length,
    })),
  list_environments: () => world.environments,
  ping_connection: ({ connectionId }) =>
    connectionId === UNREACHABLE
      ? {
          success: false,
          latencyMs: null,
          error: { kind: "cannotReach", reason: "Connection refused (os error 61)" },
        }
      : { success: true, latencyMs: connectionId === "c-local" ? 1 : between(18, 90), error: null },
  test_connection: (args) =>
    a(args).host === "fail"
      ? {
          success: false,
          redisVersion: null,
          error: { kind: "cannotReach", reason: "Connection refused" },
          sshHostKeyPinned: null,
        }
      : {
          success: true,
          redisVersion: "7.2.4",
          error: null,
          sshHostKeyPinned: null,
          probe: {
            latencyMs: 3,
            queuesFound: 12,
            queueSample: ["bull:emails", "bull:exports", "bull:image-resize", "bull:webhooks"],
            scanCapped: false,
            schedulers: 5,
            workers: 8,
          },
        },
  parse_redis_url: ({ url }) => {
    const u = new URL(String(url))
    return {
      host: u.hostname,
      port: Number(u.port || 6379),
      db: Number(u.pathname.slice(1) || 0),
      username: u.username || null,
      password: u.password || null,
      tls: u.protocol === "rediss:",
    }
  },
  add_connection: (args) => {
    const input = a(args)
    const env = world.environments[0] as Environment
    const created: Connection = {
      ...(world.connections[0] as Connection),
      id: `c-${Date.now()}`,
      name: String(input.name ?? "New connection"),
      host: String(input.host ?? "127.0.0.1"),
      port: Number(input.port ?? 6379),
      environmentId: env.id,
      groupId: env.groupId,
      groupName: env.groupName,
      envColor: env.colorToken,
      envName: env.name,
      envLabel: env.label,
      createdAt: new Date().toISOString(),
      sidebarCollapsed: false,
    }
    world.connections.push(created)
    world.queues.set(created.id, [])
    return created
  },
  update_connection: (args) =>
    Object.assign(connectionOf(a(args).id), { name: a(args).name ?? undefined }),
  duplicate_connection: (args) => {
    const source = connectionOf(a(args).id)
    const copy = { ...source, id: `c-${Date.now()}`, name: `${source.name} (copy)` }
    world.connections.push(copy)
    world.queues.set(copy.id, world.queues.get(source.id) ?? [])
    return copy
  },
  remove_connection: ({ id }) => {
    world.connections = world.connections.filter((c) => c.id !== id)
    return null
  },
  move_connection: (args) => {
    const c = connectionOf(a(args).connectionId)
    const env = world.environments.find((e) => e.id === a(args).environmentId)
    if (env)
      Object.assign(c, {
        environmentId: env.id,
        groupId: env.groupId,
        groupName: env.groupName,
        envColor: env.colorToken,
        envName: env.name,
        envLabel: env.label,
      })
    return { newlyLocked: env?.colorToken === "env-production" }
  },
  rename_environment: () => null,
  change_environment_color: () => null,
  create_environment: (args) => {
    const input = a(args)
    const group = world.groups.find((g) => g.id === input.groupId) as Group
    const id = `e-${Date.now()}`
    world.environments.push({
      id,
      groupId: group.id,
      groupName: group.name,
      name: String(input.name).toLowerCase(),
      label: String(input.name),
      colorToken: String(input.colorToken ?? "env-local"),
    })
    return id
  },
  delete_environment: () => null,
  reorder_groups: () => null,
  reorder_environments: () => null,
  create_group: (args) => {
    const g: Group = {
      id: `g-${Date.now()}`,
      name: String(a(args).name),
      displayOrder: world.groups.length,
      collapsed: false,
      connectionCount: 0,
    }
    world.groups.push(g)
    return g
  },
  rename_group: (args) =>
    Object.assign(world.groups.find((g) => g.id === a(args).id) as Group, { name: a(args).name }),
  delete_group: (args) => {
    world.groups = world.groups.filter((g) => g.id !== a(args).id)
    return null
  },
  set_group_collapsed: (args) => {
    const g = world.groups.find((x) => x.id === a(args).id)
    if (g) g.collapsed = Boolean(a(args).collapsed)
    return null
  },
  set_connection_collapsed: (args) => {
    connectionOf(a(args).connectionId).sidebarCollapsed = Boolean(a(args).collapsed)
    return null
  },

  // settings
  get_kv_setting: ({ key }) => world.kv.get(String(key)) ?? null,
  set_kv_setting: ({ key, value }) => {
    world.kv.set(String(key), String(value))
    return null
  },
  // `?update=0.3.0` on the first URL offers that version, to review the flow.
  check_for_updates: () => ({
    available: offeredUpdate !== null,
    version: offeredUpdate,
    notes: offeredUpdate
      ? "### Fixed\n- Waiting jobs were missing from the lists.\n\n### Added\n- Recent jobs on the overview pages."
      : null,
    currentVersion: "0.1.0",
    disabled: false,
    disabledReason: null,
  }),
  install_update: () => {
    throw new PreviewError(
      "updater",
      "The web preview has nothing to install — this is what a failed install looks like.",
    )
  },

  // lock
  get_connection_lock_state: (args) =>
    isLocked(String(a(args).connectionId)) ? "locked" : "unlocked",
  unlock_connection: (args) => {
    const id = String(a(args).connectionId)
    world.unlocked.add(id)
    audit(id, "unlockConnection", { kind: "connection" })
    return "unlocked"
  },
  lock_connection: (args) => {
    const id = String(a(args).connectionId)
    world.unlocked.delete(id)
    audit(id, "lockConnection", { kind: "connection" })
    return "locked"
  },

  // queues & jobs
  list_queues: (args) => queuesOf(a(args).connectionId).map(toQueue),
  list_jobs: (args) => {
    const input = a(args)
    const q = queueOf(input.connectionId, input.queueRef)
    const state = input.state as JobState
    let jobs = [...(q.jobs.get(state) ?? [])]
    const stateTotal = jobs.length
    const search = String(input.search ?? "")
      .trim()
      .toLowerCase()
    const reason = String(input.reason ?? "")
      .trim()
      .toLowerCase()
    const sort = input.sort as { field: string; descending: boolean } | null
    const queried = search !== "" || reason !== "" || sort != null
    if (search)
      jobs = jobs.filter(
        (j) => j.name.toLowerCase().includes(search) || j.jobRef.id.includes(search),
      )
    if (reason) jobs = jobs.filter((j) => j.failedReason?.toLowerCase().includes(reason))
    if (sort) {
      const key = (j: Job) =>
        sort.field === "name"
          ? j.name
          : sort.field === "queuedAt"
            ? j.timestamp
            : (j.finishedOn ?? 0) - (j.processedOn ?? 0)
      jobs.sort(
        (x, y) => (key(x) < key(y) ? -1 : key(x) > key(y) ? 1 : 0) * (sort.descending ? -1 : 1),
      )
    }
    const offset = Number(input.offset ?? 0)
    const limit = Number(input.limit ?? 50)
    const page = jobs.slice(offset, offset + limit)
    return {
      jobs: page,
      total: jobs.length,
      state,
      hasMore: offset + page.length < jobs.length,
      scanned: queried ? stateTotal : null,
      scanCapped: false,
    }
  },
  get_job: (args) => {
    const input = a(args)
    const found = findJob(queueOf(input.connectionId, input.queueRef), String(input.jobId))
    if (!found)
      throw new PreviewError(
        "jobNotFound",
        `Job not found: ${(input.queueRef as QueueRef).name}/${String(input.jobId)}`,
      )
    return found.job
  },
  get_job_logs: (args) => {
    const id = Number(a(args).jobId)
    const lines =
      id % 3 === 0
        ? []
        : [
            "Resolving template…",
            "Rendering for locale fr-BE",
            "SMTP: connecting to smtp.eu.mailer.internal:587",
            "SMTP: 250 2.0.0 OK — queued as 4F2A91",
          ]
    return { lines, total: lines.length, hasMore: false }
  },
  list_recent_jobs: (args) => {
    const input = a(args)
    const scope = input.queueRef
      ? [queueOf(input.connectionId, input.queueRef)]
      : queuesOf(input.connectionId)
    const of = (state: JobState) => scope.flatMap((q) => q.jobs.get(state) ?? [])
    const limit = Number(input.limit) || 15
    const running = of("active").sort((x, y) => (y.processedOn ?? 0) - (x.processedOn ?? 0))
    const finished = [...of("completed"), ...of("failed")].sort(
      (x, y) => (y.finishedOn ?? 0) - (x.finishedOn ?? 0),
    )
    return { jobs: [...running, ...finished].slice(0, limit), unreadableQueues: [] }
  },
  list_failed_jobs: (args) => {
    const input = a(args)
    const all = queuesOf(input.connectionId)
    let jobs = all.flatMap((q) => q.jobs.get("failed") ?? [])
    const totalFailed = jobs.length
    if (input.queue) jobs = jobs.filter((j) => j.jobRef.queue.name === input.queue)
    if (input.name) jobs = jobs.filter((j) => j.name === input.name)
    if (input.reason)
      jobs = jobs.filter((j) =>
        j.failedReason?.toLowerCase().includes(String(input.reason).toLowerCase()),
      )
    if (typeof input.signature === "string")
      jobs = jobs.filter((j) => previewSignature(j.failedReason ?? "") === input.signature)
    jobs.sort((x, y) => (y.finishedOn ?? 0) - (x.finishedOn ?? 0))
    const offset = Number(input.offset ?? 0)
    const limit = Number(input.limit ?? 50)
    return {
      jobs: jobs.slice(offset, offset + limit),
      total: jobs.length,
      totalFailed,
      queues: all
        .filter((q) => len(q, "failed") > 0)
        .map((q) => ({ queueRef: q.ref, failed: len(q, "failed") })),
      types: metrics(all, "7d").failingJobTypes,
      scanned: totalFailed,
      scanCapped: false,
      unreadableQueues: [],
    }
  },
  // Causes; the signature is `previewSignature`, a close
  // cousin of the Rust one (`bullmq_schema::failed::failure_signature`).
  list_failure_causes: (args) => {
    const input = a(args)
    const all = queuesOf(input.connectionId).filter(
      (q) => !input.queue || q.ref.name === input.queue,
    )
    const jobs = all
      .flatMap((q) => q.jobs.get("failed") ?? [])
      .sort((x, y) => (y.finishedOn ?? 0) - (x.finishedOn ?? 0))
    const now = Date.now()
    const hour = 3_600_000
    const causes = new Map<
      string,
      {
        signature: string
        sample: string
        count: number
        firstAt: number
        lastAt: number
        names: Map<string, number>
        queues: Map<string, number>
        trend: number[]
        jobs: { queue: { prefix: string; name: string }; id: string }[]
      }
    >()
    for (const j of jobs) {
      const reason = j.failedReason ?? ""
      const signature = previewSignature(reason)
      const at = j.finishedOn ?? j.timestamp
      let c = causes.get(signature)
      if (!c) {
        c = {
          signature,
          sample: reason,
          count: 0,
          firstAt: at,
          lastAt: at,
          names: new Map(),
          queues: new Map(),
          trend: Array(24).fill(0),
          jobs: [],
        }
        causes.set(signature, c)
      }
      c.count++
      c.firstAt = Math.min(c.firstAt, at)
      const bucket = Math.floor((at - (now - 24 * hour)) / hour)
      if (bucket >= 0) c.trend[Math.min(bucket, 23)]++
      c.names.set(j.name, (c.names.get(j.name) ?? 0) + 1)
      c.queues.set(j.jobRef.queue.name, (c.queues.get(j.jobRef.queue.name) ?? 0) + 1)
      c.jobs.push(j.jobRef)
    }
    const ranked = (m: Map<string, number>) =>
      [...m.entries()].sort((x, y) => y[1] - x[1] || x[0].localeCompare(y[0])).map(([k]) => k)
    return {
      causes: [...causes.values()]
        .sort((x, y) => y.count - x.count || y.lastAt - x.lastAt)
        .map(({ names, queues, ...c }) => ({
          ...c,
          jobNames: ranked(names).slice(0, 5),
          queueNames: ranked(queues),
        })),
      scanned: jobs.length,
      scanCapped: false,
      totalFailed: jobs.length,
      unreadableQueues: [],
    }
  },
  add_job: (args) => {
    const input = a(args)
    guardWrite(input.connectionId)
    const q = queueOf(input.connectionId, input.queueRef)
    const spec = input.job as {
      name: string
      data: string
      delayMs: number
      priority: number
      attempts: number | null
    }
    const state: JobState =
      spec.delayMs > 0
        ? "delayed"
        : spec.priority > 0
          ? "prioritized"
          : q.paused
            ? "paused"
            : "waiting"
    const id = String(9000 + q.nextId++)
    const job = makeJob(q.ref, id, spec.name, state, Date.now())
    Object.assign(job, {
      data: spec.data,
      timestamp: Date.now(),
      delay: spec.delayMs || null,
      attemptsMade: 0,
      processedOn: null,
      finishedOn: null,
    })
    q.jobs.set(state, [job, ...(q.jobs.get(state) ?? [])])
    audit(
      String(input.connectionId),
      "addJob",
      { kind: "job", data: { queueName: q.ref.name, jobId: id } },
      `${spec.name} → ${state}`,
    )
    return { jobId: id, state }
  },
  retry_job: (args) =>
    jobAction(a(args), "failed", "waiting", "retry", "retryJob", {
      failedReason: null,
      finishedOn: null,
      processedOn: null,
      stacktrace: [],
    }),
  promote_job: (args) =>
    jobAction(a(args), "delayed", "waiting", "promote", "promoteJob", { delay: null }),
  remove_job: (args) => {
    const input = a(args)
    guardWrite(input.connectionId)
    const q = queueOf(input.connectionId, input.queueRef)
    const found = findJob(q, String(input.jobId))
    if (!found)
      throw new PreviewError("jobNotFound", `Job not found: ${q.ref.name}/${String(input.jobId)}`)
    q.jobs.set(
      found.state,
      (q.jobs.get(found.state) ?? []).filter((j) => j !== found.job),
    )
    audit(String(input.connectionId), "removeJob", {
      kind: "job",
      data: { queueName: q.ref.name, jobId: String(input.jobId) },
    })
    return { jobId: input.jobId, newState: null, action: "remove" }
  },
  bulk_retry_jobs: (args) =>
    bulk(a(args), "retry", (q, id) =>
      moveJob(q, id, "failed", "waiting", {
        failedReason: null,
        finishedOn: null,
        processedOn: null,
      }),
    ),
  bulk_promote_jobs: (args) =>
    bulk(a(args), "promote", (q, id) => moveJob(q, id, "delayed", "waiting", { delay: null })),
  bulk_remove_jobs: (args) =>
    bulk(a(args), "remove", (q, id) => {
      const found = findJob(q, id)
      if (!found) return false
      q.jobs.set(
        found.state,
        (q.jobs.get(found.state) ?? []).filter((j) => j !== found.job),
      )
      return true
    }),
  pause_queue: (args) => setPaused(a(args), true),
  resume_queue: (args) => setPaused(a(args), false),

  // maintenance
  preview_queue_maintenance: (args) => {
    const input = a(args)
    const q = queueOf(input.connectionId, input.queueRef)
    const op = input.operation as {
      kind: string
      includeDelayed?: boolean
      state?: JobState
      olderThanMs?: number
    }
    if (op.kind === "drain") {
      const affected =
        len(q, "waiting") +
        len(q, "paused") +
        len(q, "prioritized") +
        (op.includeDelayed ? len(q, "delayed") : 0)
      const spared = op.includeDelayed ? q.schedulers.filter((s) => !s.paused).length : 0
      return { affected: Math.max(0, affected - spared), sparedSchedulerJobs: spared }
    }
    if (op.kind === "clean") {
      const cutoff = Date.now() - (op.olderThanMs ?? 0)
      return {
        affected: (q.jobs.get(op.state as JobState) ?? []).filter(
          (j) => (j.finishedOn ?? 0) <= cutoff,
        ).length,
        sparedSchedulerJobs: 0,
      }
    }
    return { affected: len(q, "failed"), sparedSchedulerJobs: 0 }
  },
  drain_queue: (args) => {
    const input = a(args)
    guardWrite(input.connectionId)
    const q = queueOf(input.connectionId, input.queueRef)
    const states: JobState[] = [
      "waiting",
      "paused",
      "prioritized",
      ...(input.includeDelayed ? (["delayed"] as JobState[]) : []),
    ]
    let affected = 0
    for (const s of states) {
      affected += len(q, s)
      q.jobs.set(s, [])
    }
    audit(
      String(input.connectionId),
      "drainQueue",
      { kind: "queue", data: { queueName: q.ref.name } },
      `${affected} removed`,
    )
    return { action: "drain", affected, skipped: 0, cancelled: false }
  },
  clean_queue: (args) => {
    const input = a(args)
    guardWrite(input.connectionId)
    const q = queueOf(input.connectionId, input.queueRef)
    const state = input.state as JobState
    const cutoff = Date.now() - Number(input.olderThanMs ?? 0)
    const before = len(q, state)
    q.jobs.set(
      state,
      (q.jobs.get(state) ?? []).filter((j) => (j.finishedOn ?? 0) > cutoff),
    )
    const affected = before - len(q, state)
    audit(
      String(input.connectionId),
      "cleanQueue",
      { kind: "queue", data: { queueName: q.ref.name } },
      `${affected} removed`,
    )
    return { action: "clean", affected, skipped: 0, cancelled: false }
  },
  retry_all_failed: (args) => {
    const input = a(args)
    guardWrite(input.connectionId)
    const q = queueOf(input.connectionId, input.queueRef)
    const failed = q.jobs.get("failed") ?? []
    for (const job of failed)
      Object.assign(job, {
        state: "waiting",
        failedReason: null,
        finishedOn: null,
        processedOn: null,
        stacktrace: [],
      })
    q.jobs.set("waiting", [...failed, ...(q.jobs.get("waiting") ?? [])])
    q.jobs.set("failed", [])
    audit(
      String(input.connectionId),
      "retryAllFailed",
      { kind: "queue", data: { queueName: q.ref.name } },
      `${failed.length} retried`,
    )
    return { action: "retry", affected: failed.length, skipped: 0, cancelled: false }
  },

  // schedulers
  list_cron_jobs: (args) => queueOf(a(args).connectionId, a(args).queueRef).schedulers,
  preview_schedule: ({ schedule }) => ({ nextRunsMs: nextRuns(schedule as Args) }),
  // Runs in a window. Minute by minute against the pattern's
  // fields (*, */n, n, a-b, lists): a preview, not a cron engine.
  schedule_runs_within: (args) => {
    const input = a(args)
    const schedule = input.schedule as Args
    const end = Date.now() + Number(input.horizonMs ?? DAY)
    const max = Math.min(Number(input.max ?? 500), 500)
    const runs: number[] = []
    if (schedule.kind === "every") {
      const every = Number(schedule.everyMs)
      let t = Number(input.firstMs ?? Date.now() + every)
      while (t <= Date.now()) t += every
      while (t <= end && runs.length < max) {
        runs.push(t)
        t += every
      }
      return { nextRunsMs: runs }
    }
    const fields = String(schedule.pattern ?? "")
      .trim()
      .split(/\s+/)
    const [m, h, dom, mon, dow] = fields.length === 6 ? fields.slice(1) : fields
    const match = (field: string | undefined, v: number) =>
      (field ?? "*").split(",").some((part) => {
        const [range, step] = part.split("/")
        const n = Number(step ?? 1)
        if (range === "*") return v % n === 0
        const [lo, hi] = (range ?? "").split("-").map(Number)
        return v >= (lo ?? 0) && v <= (hi ?? lo ?? 0) && (v - (lo ?? 0)) % n === 0
      })
    const t = new Date(Date.now())
    t.setSeconds(0, 0)
    for (
      t.setMinutes(t.getMinutes() + 1);
      t.getTime() <= end && runs.length < max;
      t.setMinutes(t.getMinutes() + 1)
    ) {
      if (
        match(m, t.getMinutes()) &&
        match(h, t.getHours()) &&
        match(dom, t.getDate()) &&
        match(mon, t.getMonth() + 1) &&
        match(dow, t.getDay())
      ) {
        runs.push(t.getTime())
      }
    }
    return { nextRunsMs: runs }
  },
  upsert_scheduler: (args) => {
    const input = a(args)
    guardWrite(input.connectionId)
    const q = queueOf(input.connectionId, input.queueRef)
    const spec = input.scheduler as { id: string; jobName: string; schedule: Args; data: string }
    const runs = nextRuns(spec.schedule)
    const existing = q.schedulers.find((s) => s.key === spec.id)
    const next: CronJob = {
      key: spec.id,
      name: spec.jobName,
      pattern: spec.schedule.kind === "pattern" ? String(spec.schedule.pattern) : null,
      every: spec.schedule.kind === "every" ? Number(spec.schedule.everyMs) : null,
      tz: spec.schedule.kind === "pattern" ? ((spec.schedule.tz as string | null) ?? null) : null,
      offset: 0,
      iterationCount: existing?.iterationCount ?? 1,
      nextRun: runs[0] as number,
      endDate: null,
      template: spec.data === "{}" ? null : spec.data,
      paused: false,
    }
    q.schedulers = existing
      ? q.schedulers.map((s) => (s.key === spec.id ? next : s))
      : [...q.schedulers, next]
    audit(String(input.connectionId), "upsertScheduler", {
      kind: "scheduler",
      data: { queueName: q.ref.name, schedulerKey: spec.id },
    })
    return {
      schedulerId: spec.id,
      nextRunMs: next.nextRun,
      jobId: `repeat:${spec.id}:${next.nextRun}`,
    }
  },
  remove_scheduler: (args) => {
    const input = a(args)
    guardWrite(input.connectionId)
    const q = queueOf(input.connectionId, input.queueRef)
    q.schedulers = q.schedulers.filter((s) => s.key !== input.schedulerKey)
    audit(String(input.connectionId), "removeScheduler", {
      kind: "scheduler",
      data: { queueName: q.ref.name, schedulerKey: String(input.schedulerKey) },
    })
    return { paused: false, alreadyInState: false }
  },
  pause_scheduler: (args) => toggleScheduler(a(args), true),
  resume_scheduler: (args) => toggleScheduler(a(args), false),

  // workers, flows, metrics
  count_queue_workers: (args) => {
    const input = a(args)
    // Production forbids CLIENT LIST, as in `list_workers` below.
    if (input.connectionId === "c-prod") return { supported: false, counts: [] }
    const refs = (input.queueRefs ?? []) as QueueRef[]
    return {
      supported: true,
      counts: refs.map((queueRef) => ({
        queueRef,
        workers: queueOf(input.connectionId, queueRef).workers.length,
      })),
    }
  },
  list_workers: (args) => {
    const input = a(args)
    if (input.connectionId === "c-prod") {
      return {
        supported: false,
        unsupportedReason: "NOPERM this user has no permissions to run the 'client|list' command",
        workers: [],
      }
    }
    return {
      supported: true,
      unsupportedReason: null,
      workers: queueOf(input.connectionId, input.queueRef).workers,
    }
  },
  get_flow: (args) =>
    flowFor(a(args).connectionId, a(args).queueRef as QueueRef, String(a(args).jobId)),
  list_flow_roots: (args) => {
    const q = queueOf(a(args).connectionId, a(args).queueRef)
    const parent = q.jobs.get("waitingChildren")?.[0]
    return {
      roots: parent
        ? [
            {
              jobRef: parent.jobRef,
              name: parent.name,
              state: parent.state,
              timestamp: parent.timestamp,
              childrenTotal: 2,
              childrenPending: 0,
              childrenCompleted: 1,
              childrenFailed: 1,
              blocked: true,
            },
          ]
        : [],
      scanned: len(q, "completed") + len(q, "failed"),
      capped: false,
    }
  },
  get_queue_metrics: (args) =>
    metrics([queueOf(a(args).connectionId, a(args).queueRef)], String(a(args).range ?? "24h")),
  get_connection_metrics: (args) =>
    metrics(queuesOf(a(args).connectionId), String(a(args).range ?? "24h")),
  // The approval behind an audited agent write: asked by the
  // entry's agent a few seconds before it, approved 12 s later.
  get_approval_record: (args) => {
    const id = String((args as { approvalId?: string }).approvalId)
    const entry = world.audit.find((e) => e.approvalId === id)
    if (!entry) return null
    const decided = Date.parse(entry.createdAt) - 2_000
    return {
      id,
      createdAt: new Date(decided - 12_000).toISOString(),
      decidedAt: new Date(decided).toISOString(),
      status: "consumed",
      clientHint: entry.agentClient,
      tool: "bulk_remove_jobs",
      reason: null,
      argumentsJson: JSON.stringify({
        connectionId: entry.connectionId,
        queueRef: { prefix: "bull", name: "emails" },
        jobIds: ["1081", "1082", "1083"],
      }),
    }
  },
  // What needs the user across connections, from the same
  // world as the other stubs: causes, paused / worker-less queues, offline.
  list_attention: () => {
    const items: unknown[] = []
    for (const c of world.connections) {
      const base = {
        connectionId: c.id,
        connectionName: c.name,
        envColor: c.envColor,
        envLabel: c.envLabel,
        queueName: null as string | null,
        sample: null as string | null,
        sinceMs: null as number | null,
        trend: [] as number[],
        jobs: [] as unknown[],
      }
      if (c.id === UNREACHABLE) {
        items.push({
          ...base,
          key: `offline:${c.id}`,
          kind: "offline",
          reason: "Connection refused (os error 61)",
          count: 0,
        })
        continue
      }
      const causes = commands.list_failure_causes?.({
        args: { connectionId: c.id, queue: null },
      }) as {
        causes: Array<{
          signature: string
          sample: string
          count: number
          firstAt: number
          queueNames: string[]
          trend: number[]
          jobs: unknown[]
        }>
      }
      for (const cause of causes.causes.slice(0, 10)) {
        items.push({
          ...base,
          key: `failing:${c.id}:${cause.signature}`,
          kind: "failing",
          queueName: cause.queueNames.length === 1 ? cause.queueNames[0] : null,
          reason: cause.signature,
          sample: cause.sample,
          count: cause.count,
          sinceMs: cause.firstAt,
          trend: cause.trend,
          jobs: cause.jobs,
        })
      }
      for (const q of queuesOf(c.id)) {
        const waiting = len(q, "waiting") + len(q, "prioritized") + len(q, "paused")
        if (q.paused) {
          items.push({
            ...base,
            key: `paused:${c.id}:${q.ref.name}`,
            kind: "stuck",
            queueName: q.ref.name,
            reason: "paused",
            count: waiting,
          })
        } else if (waiting > 0 && c.id !== "c-prod" && q.workers.length === 0) {
          items.push({
            ...base,
            key: `noworker:${c.id}:${q.ref.name}`,
            kind: "stuck",
            queueName: q.ref.name,
            reason: "noWorker",
            count: waiting,
          })
        }
      }
    }
    const env = (e: string) => (e === "env-production" ? 0 : e === "env-staging" ? 1 : 2)
    const kind = (k: string) => (k === "offline" ? 0 : k === "failing" ? 1 : 2)
    type Item = { envColor: string; kind: string; count: number; key: string }
    ;(items as Item[]).sort(
      (x, y) =>
        env(x.envColor) - env(y.envColor) ||
        kind(x.kind) - kind(y.kind) ||
        y.count - x.count ||
        x.key.localeCompare(y.key),
    )
    return { items, connectionsChecked: world.connections.length }
  },
  // Live events: the finished jobs as events, newest first,
  // then whatever the preview world does next (nothing: it is still).
  read_queue_events: (args) => {
    const input = a(args)
    const cursors = (input.cursors ?? []) as Array<{ queueName: string; lastId: string }>
    const events = queuesOf(input.connectionId).flatMap((q) =>
      (["completed", "failed"] as const).flatMap((state) =>
        (q.jobs.get(state) ?? [])
          .filter((j) => j.finishedOn)
          .map((j) => ({
            queueName: q.ref.name,
            streamId: `${j.finishedOn}-0`,
            atMs: j.finishedOn ?? 0,
            event: state,
            jobId: j.jobRef.id,
            jobName: j.name,
            failedReason: state === "failed" ? j.failedReason : null,
          })),
      ),
    )
    events.sort((x, y) => y.atMs - x.atMs)
    const fresh = cursors.length > 0 ? [] : events.slice(0, 20)
    return {
      events: fresh,
      cursors: queuesOf(input.connectionId).map((q) => ({
        queueName: q.ref.name,
        lastId: `${Date.now()}-0`,
      })),
      sampled: false,
    }
  },
  // The server behind a connection (fixed: no random draw).
  get_server_info: () => ({
    redisVersion: "7.2.4",
    latencyMs: 2,
    usedMemoryBytes: 48_234_496,
    maxMemoryBytes: 268_435_456,
    opsPerSec: 37,
    connectedClients: 12,
  }),
  // per-queue series from the finished jobs; `reports` records
  // no metrics, to show "metrics off".
  get_throughput: (args) => {
    const input = a(args)
    const range = String(input.range ?? "24h")
    const span = RANGE_MS[range] ?? DAY
    const count = range === "1h" ? 12 : range === "6h" ? 12 : 24
    const bucketMs = Math.ceil(span / count)
    const now = Date.now()
    const start = now - span
    const completed = new Array<number>(count).fill(0)
    const failed = new Array<number>(count).fill(0)
    const queues = queuesOf(input.connectionId).map((q) => {
      if (q.ref.name === "reports") {
        return { queueRef: q.ref, metricsOff: true, completed: [], failed: [] }
      }
      const c = new Array<number>(count).fill(0)
      const f = new Array<number>(count).fill(0)
      for (const [state, series] of [
        ["completed", c],
        ["failed", f],
      ] as const) {
        for (const job of q.jobs.get(state) ?? []) {
          if (!job.finishedOn || job.finishedOn <= start) continue
          const b = Math.min(count - 1, Math.floor((job.finishedOn - start) / bucketMs))
          series[b] = (series[b] ?? 0) + 1
        }
      }
      c.forEach((v, i) => {
        completed[i] = (completed[i] ?? 0) + v
      })
      f.forEach((v, i) => {
        failed[i] = (failed[i] ?? 0) + v
      })
      return { queueRef: q.ref, metricsOff: false, completed: c, failed: f }
    })
    return {
      range,
      bucketMs,
      bucketStartsMs: completed.map((_, i) => start + i * bucketMs),
      queues,
      completed,
      failed,
      metricsOffQueues: queues.filter((q) => q.metricsOff).length,
    }
  },

  // audit
  list_audit_log_entries: (args) => {
    const f = a(args)
    return world.audit
      .filter(
        (e) =>
          (!f.connectionId || e.connectionId === f.connectionId) &&
          (!f.action || e.action === f.action) &&
          (!f.source || e.source === f.source),
      )
      .slice(Number(f.offset ?? 0), Number(f.offset ?? 0) + Number(f.limit ?? 100))
  },
  export_audit_log: (args) => ({
    content: JSON.stringify(world.audit, null, 2),
    suggestedFilename: `ranch-audit.${String(a(args).format ?? "json")}`,
  }),

  // MCP
  get_mcp_safety_settings: () => world.safety,
  set_mcp_safety_settings: ({ settings }) => {
    world.safety = settings as McpSafetySettings
    return world.safety
  },
  set_connection_mcp_readonly: (args) => {
    connectionOf(a(args).connectionId).mcpReadonly = (a(args).readonly as boolean | null) ?? null
    return null
  },
  list_pending_approvals: () => world.approvals,
  // The Map: flow links from the demo jobs' parent keys.
  // No event streams in the fake world: the preview polls.
  follow_connection_events: () => false,
  get_topology: (args) => {
    const queues = queuesOf(a(args).connectionId)
    const links = new Map<string, { parent: QueueRef; child: QueueRef; jobs: number }>()
    for (const q of queues) {
      for (const jobs of q.jobs.values()) {
        for (const job of jobs) {
          const parent = job.parentKey?.split(":").slice(1, -1).join(":")
          if (!parent) continue
          const key = `${parent}>${q.ref.name}`
          const link = links.get(key) ?? {
            parent: { prefix: q.ref.prefix, name: parent },
            child: q.ref,
            jobs: 0,
          }
          link.jobs += 1
          links.set(key, link)
        }
      }
    }
    return {
      links: [...links.values()],
      queues: queues.map((q) => {
        const waiting = q.jobs.get("waiting") ?? []
        const done = (q.jobs.get("completed") ?? []).length + (q.jobs.get("failed") ?? []).length
        return {
          queueRef: q.ref,
          addedPerMinute: done / 60,
          addedCapped: false,
          oldestWaitingMs: waiting.length > 0 ? Math.min(...waiting.map((j) => j.timestamp)) : null,
        }
      }),
    }
  },
  // A Timeline window, from the demo throughput.
  summarize_window: (args) => {
    const input = a(args)
    const from = Number(input.fromMs)
    const to = Number(input.toMs)
    const rangeFrom = Number(input.rangeFromMs)
    const t = commands.get_throughput?.({
      args: { connectionId: input.connectionId, range: "7d" },
    }) as
      | { bucketMs: number; bucketStartsMs: number[]; completed: number[]; failed: number[] }
      | undefined
    let done = 0
    let failed = 0
    let usualDone = 0
    let usualFailed = 0
    t?.bucketStartsMs.forEach((start, i) => {
      const end = start + t.bucketMs
      if (end < rangeFrom) return
      const inWindow = end > from && start < to
      if (inWindow) {
        done += t.completed[i] ?? 0
        failed += t.failed[i] ?? 0
      } else {
        usualDone += t.completed[i] ?? 0
        usualFailed += t.failed[i] ?? 0
      }
    })
    const scale = (to - from) / Math.max(1, Date.now() - rangeFrom - (to - from))
    const causes = (
      commands.list_failure_causes?.({ args: { connectionId: input.connectionId, queue: null } }) as
        | { causes: Array<{ signature: string; sample: string; jobs: Array<unknown> }> }
        | undefined
    )?.causes
      .slice(0, 3)
      .map((c, i) => ({
        signature: c.signature,
        sample: c.sample,
        count: Math.max(1, Math.round(failed / (i + 2))),
        jobs: c.jobs.slice(0, 5),
      }))
    return {
      fromMs: from,
      toMs: to,
      done,
      failed,
      usualDone: usualDone * scale,
      usualFailed: usualFailed * scale,
      waitP95Ms: 4_800,
      usualWaitP95Ms: 1_200,
      causes: failed > 0 ? (causes ?? []) : [],
      scanned: done + failed,
      scanCapped: false,
    }
  },
  // Ask AI: the preview masks like the default policy and
  // pretends to launch (nothing to start in a browser).
  share_for_agents: ({ json }) =>
    String(json).replace(
      /"([^"]*(?:email|phone|token|password|secret|iban)[^"]*|card[^"]*)":\s*("(?:[^"\\]|\\.)*"|[\d.]+)/gi,
      '"$1":"•••"',
    ),
  launch_claude_code: () => null,
  open_agent_app: () => null,
  // agents' notes: one on the biggest cause of c-prod/emails.
  list_cause_notes: (args) => {
    const input = a(args)
    const conn = world.connections.find((c) => c.id === "c-prod")
    if (!conn) return []
    if (input.connectionId && input.connectionId !== conn.id) return []
    const causes = commands.list_failure_causes?.({
      args: { connectionId: conn.id, queue: "emails" },
    }) as { causes: Array<{ signature: string }> } | undefined
    const signature = causes?.causes[0]?.signature
    if (signature === undefined || previewDismissed.has("note-1")) {
      return input.includeDismissed && signature !== undefined
        ? [demoNote(conn, signature, true)]
        : []
    }
    return [demoNote(conn, signature, false)]
  },
  dismiss_cause_note: ({ noteId }) => {
    previewDismissed.add(String(noteId))
    return null
  },
  // Agents › Approvals, earlier decisions.
  list_approval_history: () => {
    const conn = world.connections.find((c) => c.id === "c-prod") ?? world.connections[0]
    if (!conn) return []
    const at = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString()
    return [
      ["h-1", 12, "consumed", "claude-ai", "bulk_retry_jobs", null],
      ["h-2", 95, "denied", "cursor", "drain_queue", "Not on production, ask me first"],
      ["h-3", 300, "expired", "claude-ai", "remove_job", null],
      ["h-4", 1440, "consumed", "claude-code", "retry_job", null],
    ].map(([id, minutes, status, client, tool, reason]) => ({
      id: String(id),
      createdAt: at(Number(minutes)),
      decidedAt: at(Number(minutes) - 1),
      status: String(status),
      clientHint: String(client),
      tool: String(tool),
      reason: reason === null ? null : String(reason),
      connectionId: conn.id,
      connectionName: conn.name,
    }))
  },
  set_ai_features: ({ enabled }) => {
    world.kv.set("ai_enabled", enabled ? "true" : "false")
    if (!enabled) world.approvals = []
    return Boolean(enabled)
  },
  decide_pending_approval: (args) => {
    world.approvals = world.approvals.filter((p) => p.id !== a(args).id)
    return "decided"
  },
  list_mcp_activity: (args) => {
    const f = (a(args).filter ?? {}) as Args
    const events = world.activity.filter(
      (e) =>
        (!f.kind || e.kind === f.kind) &&
        (!f.status || e.status === f.status) &&
        (!f.sessionId || e.sessionId === f.sessionId),
    )
    return { events, total: events.length }
  },
  list_mcp_activity_sessions: () => [
    {
      sessionId: "s-now",
      clientHint: "Claude Desktop",
      startedAt: new Date(Date.now() - 8 * MIN).toISOString(),
      lastAt: new Date(Date.now() - 2 * MIN).toISOString(),
      total: 5,
      writes: 3,
      notSuccessful: 2,
    },
    {
      sessionId: "s-morning",
      clientHint: "Cursor",
      startedAt: new Date(Date.now() - 5 * HOUR - 3 * MIN).toISOString(),
      lastAt: new Date(Date.now() - 5 * HOUR).toISOString(),
      total: 3,
      writes: 1,
      notSuccessful: 1,
    },
  ],
  export_mcp_activity: () => JSON.stringify(world.activity, null, 2),
  clear_mcp_activity: () => {
    const n = world.activity.length
    world.activity = []
    return n
  },
  get_mcp_installer_status: () => world.installers,
  install_mcp_for_claude_desktop: () => install("claudeDesktop"),
  install_mcp_for_cursor: () => install("cursor"),
  install_mcp_for_client: ({ client }) => install(String(client)),
  get_mcp_manual_snippet: () => ({
    binaryPath: "/Applications/Ranch.app/Contents/MacOS/ranch-mcp",
    jsonSnippet: JSON.stringify(
      { mcpServers: { ranch: { command: "/Applications/Ranch.app/Contents/MacOS/ranch-mcp" } } },
      null,
      2,
    ),
  }),
}

function install(client: string) {
  const status = world.installers[client] as McpInstallerStatus
  if (status.cliFound === false) {
    status.error = "The client's command-line tool was not found"
    return status
  }
  status.installed = true
  return status
}

function jobAction(
  input: Args,
  from: JobState,
  to: JobState,
  action: string,
  auditAction: AuditLogEntry["action"],
  patch: Partial<Job>,
) {
  guardWrite(input.connectionId)
  const q = queueOf(input.connectionId, input.queueRef)
  const target = to === "waiting" && q.paused ? "paused" : to
  if (!moveJob(q, String(input.jobId), from, target, patch)) {
    throw new PreviewError("invalidJobState", `Job ${String(input.jobId)} is not ${from}`)
  }
  audit(String(input.connectionId), auditAction, {
    kind: "job",
    data: { queueName: q.ref.name, jobId: String(input.jobId) },
  })
  return { jobId: input.jobId, newState: target, action }
}

function bulk(input: Args, action: string, apply: (q: QueueWorld, id: string) => boolean) {
  guardWrite(input.connectionId)
  const q = queueOf(input.connectionId, input.queueRef)
  const ids = (input.jobIds as string[]) ?? []
  const failed = ids
    .filter((id) => !apply(q, id))
    .map((jobId) => ({ jobId, reason: "not in the expected state" }))
  return {
    action,
    total: ids.length,
    succeeded: ids.length - failed.length,
    failed,
    cancelled: false,
  }
}

function setPaused(input: Args, paused: boolean) {
  guardWrite(input.connectionId)
  const q = queueOf(input.connectionId, input.queueRef)
  const already = q.paused === paused
  q.paused = paused
  const [from, to]: [JobState, JobState] = paused ? ["waiting", "paused"] : ["paused", "waiting"]
  q.jobs.set(to, [
    ...(q.jobs.get(from) ?? []).map((j) => ({ ...j, state: to })),
    ...(q.jobs.get(to) ?? []),
  ])
  q.jobs.set(from, [])
  audit(String(input.connectionId), paused ? "pauseQueue" : "resumeQueue", {
    kind: "queue",
    data: { queueName: q.ref.name },
  })
  return { isPaused: paused, alreadyInState: already }
}

function toggleScheduler(input: Args, paused: boolean) {
  guardWrite(input.connectionId)
  const q = queueOf(input.connectionId, input.queueRef)
  const s = q.schedulers.find((x) => x.key === input.schedulerKey)
  if (!s)
    throw new PreviewError(
      "schedulerNotFound",
      `Scheduler not found: ${String(input.schedulerKey)}`,
    )
  const already = s.paused === paused
  s.paused = paused
  return { paused, alreadyInState: already }
}

// ── installation ─────────────────────────────────────────────────────────
declare global {
  interface Window {
    ranchPreview?: {
      requestApproval: (tool?: string) => void
      reset: () => void
      world: () => World
    }
  }
}

/** Read once, at start: the router drops search params it does not know. */
let offeredUpdate: string | null = null

export function installWebPreview() {
  offeredUpdate = new URLSearchParams(window.location.search).get("update")
  // The shim answers the Tauri plugins; app commands come here.
  installTauriShim(async (cmd, args) => {
    const handler = commands[cmd]
    if (!handler) {
      console.warn(`[ranch web] no preview for \`${cmd}\``, args)
      throw new PreviewError("invalidInput", `\`${cmd}\` is not available in the web preview`)
    }
    // A little latency, so loading states exist.
    await new Promise((resolve) => setTimeout(resolve, 60 + Math.random() * 120))
    try {
      return structuredClone(handler(args as Args))
    } catch (error) {
      if (error instanceof PreviewError) {
        throw { kind: error.kind, message: error.message, docUrl: error.docUrl }
      }
      throw error
    }
  })
  window.ranchPreview = {
    requestApproval,
    reset: () => {
      world = buildScenario()
    },
    world: () => world,
  }
  console.info(
    "[ranch web] browser preview with an in-memory BullMQ world — `ranchPreview.requestApproval()` to see an agent approval",
  )
}

/** The web preview's failure signature (see `list_failure_causes`). */
function previewSignature(reason: string): string {
  const line = reason.split("\n").find((l) => l.trim() !== "") ?? ""
  return line
    .trim()
    .replace(/\b[a-z][a-z0-9+.-]*:\/\/[^\s"'`)\]>,]+/gi, "<url>")
    .replace(/"[^"]*"|`[^`]*`|(?<![\p{L}\p{N}])'[^']*'(?![\p{L}\p{N}])/gu, "<str>")
    .replace(/\b\d{4}-\d{2}-\d{2}[T ][\d:.]+(Z|[+-]\d{2}:?\d{2})?/g, "<time>")
    .replace(/\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/gi, "<id>")
    .replace(/\b\d{1,3}(\.\d{1,3}){3}(:\d+)?\b/g, "<ip>")
    .replace(/\b0x[0-9a-f]+\b/gi, "<hex>")
    .replace(/\b(?=[0-9a-f]*[a-f])(?=[0-9a-f]*\d)[0-9a-f]{12,}\b/gi, "<hex>")
    .replace(/(?<![\p{L}\d_-])\d+(\.\d+)?(?![\p{L}\d])/gu, "<n>")
    .replace(/\d{2,}/g, "<n>")
    .replace(/\s+/g, " ")
}
