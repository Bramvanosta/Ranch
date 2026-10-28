/**
 * The demo world as it starts: "Lumen Studios" and its connections, queues
 * with jobs in every state, schedulers, a flow, workers, audit entries and
 * agent activity. Deterministic, so screenshots are stable.
 */
import { bullmqConnection, redisEndpoint } from "@/domain/bullmqEndpoint"
import type {
  ActivityEvent,
  AuditLogEntry,
  BullmqRedisEndpoint,
  Connection,
  Environment,
  Group,
  Job,
  JobState,
  McpInstallerStatus,
  QueueId,
} from "@/lib/tauri/bindings"
import { DAY, HOUR, MIN, type QueueWorld, type World } from "./types"

// ── deterministic pseudo-random, so screenshots are stable ────────────────
let seed = 42
export function rnd(): number {
  seed = (seed * 1664525 + 1013904223) % 4294967296
  return seed / 4294967296
}
export const pick = <T>(list: readonly T[]): T => list[Math.floor(rnd() * list.length)] as T
export const between = (min: number, max: number) => Math.floor(min + rnd() * (max - min))

export const FAILURES = [
  "connect ETIMEDOUT 10.0.4.12:5432",
  "Request failed with status code 503",
  'ValidationError: "email" must be a valid email',
  "RateLimitError: 429 Too Many Requests, retry after 30s",
  "TypeError: Cannot read properties of undefined (reading 'id')",
]

export function makeJob(
  queue: { id: QueueId; name: string },
  id: string,
  name: string,
  state: JobState,
  now: number,
): Job {
  const queuedAgo = between(2 * MIN, 3 * DAY)
  const timestamp = now - queuedAgo
  const started = timestamp + between(200, 40_000)
  const took = between(40, 9_000)
  const finished = state === "completed" || state === "failed"
  const failed = state === "failed"
  // One delayed job in three is between two attempts: it failed once and
  // waits for its backoff; BullMQ keeps the reason and the trace on it.
  const retrying = state === "delayed" && between(0, 2) === 0
  const reason = failed || retrying ? pick(FAILURES) : null
  // BullMQ stores one whole `err.stack` per failed attempt.
  const trace = (n: number) =>
    `Error: ${reason}\n    at sendEmail (/app/dist/workers/email.js:${41 + n}:13)\n    at async Worker.processFn (/app/dist/workers/index.js:18:9)\n    at async Worker.callProcessJob (/app/node_modules/bullmq/dist/cjs/classes/worker.js:178:21)\n    at async Worker.retryIfFailed (/app/node_modules/bullmq/dist/cjs/classes/worker.js:581:24)\n    at process.processTicksAndRejections (node:internal/process/task_queues:95:5)`
  return {
    jobRef: { queueId: queue.id, queueName: queue.name, id },
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

export function makeQueue(
  prefix: string,
  name: string,
  names: string[],
  counts: Partial<Record<JobState, number>>,
  now: number,
): QueueWorld {
  // The demo makes its queues' ids as BullMQ does: the key base.
  const ref = { id: `${prefix}:${name}`, name }
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
  return { ...ref, paused: false, jobs, schedulers: [], workers: [], nextId }
}

/**
 * A queue shaped like a real one that broke the first table design: one job
 * name for every job, long custom ids built from a uuid that is also in the
 * payload, and multi-line Prisma failures whose message is on the last line.
 */
export function enrichmentQueue(now: number): QueueWorld {
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

export function buildWorld(): World {
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
      name: "Atelier Nord (client)",
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
    endpoint: BullmqRedisEndpoint,
    over: Partial<Connection> = {},
  ): Connection => {
    const e = environments.find((x) => x.id === environmentId) as Environment
    return {
      id,
      environmentId,
      groupId: e.groupId,
      name,
      ...bullmqConnection(endpoint),
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
    conn("c-local", "e-t-local", "lumen-api", redisEndpoint({ host: "127.0.0.1", port: 6390 })),
    conn(
      "c-staging",
      "e-t-staging",
      "lumen-api",
      redisEndpoint({ host: "redis.staging.lumen.internal", tls: true }),
    ),
    conn(
      "c-prod",
      "e-t-prod",
      "lumen-api",
      redisEndpoint({
        host: "redis.prod.lumen.internal",
        tls: true,
        username: "ranch",
        sshTunnel: {
          host: "bastion.lumen.example",
          port: 22,
          user: "deploy",
          auth: { kind: "keyFile", keyFilePath: "~/.ssh/id_ed25519" },
          jumpHost: null,
        },
      }),
      { mcpReadonly: true },
    ),
    conn(
      "c-side",
      "e-s-local",
      "playground",
      redisEndpoint({ host: "localhost", port: 6380, db: 2 }),
    ),
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
  const orders = local.find((q) => q.name === "orders") as QueueWorld
  const parent = makeJob(orders, "flow-1", "assemble-order", "waitingChildren", now)
  orders.jobs.set("waitingChildren", [parent])
  const emailsQ = local.find((q) => q.name === "emails") as QueueWorld
  const childOk = (emailsQ.jobs.get("completed") as Job[])[0] as Job
  const childBad = (emailsQ.jobs.get("failed") as Job[])[0] as Job
  childOk.parentKey = "bull:orders:flow-1"
  childBad.parentKey = "bull:orders:flow-1"
  // A child with a long name: the flow graph must wrap it, never cut it.
  const childLong = (emailsQ.jobs.get("completed") as Job[])[1] as Job
  childLong.name = "render-and-attach-the-monthly-statement-pdf-for-every-customer"
  childLong.parentKey = "bull:orders:flow-1"

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
      groupName: "Lumen Studios",
      environmentName: "Staging",
      approvalId: null,
      ...over,
    })
  auditRow(
    3 * MIN,
    "retryAllFailed",
    { kind: "queue", data: { queueId: "bull:emails", queueName: "emails" } },
    { outcomeReason: "111 retried" },
  )
  auditRow(
    9 * MIN,
    "addJob",
    { kind: "job", data: { queueId: "bull:emails", queueName: "emails", jobId: "4811" } },
    { outcomeReason: "send-email → waiting" },
  )
  auditRow(
    22 * MIN,
    "removeJob",
    { kind: "bulkJobs", data: { queueId: "bull:emails", queueName: "emails", jobCount: 12 } },
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
    {
      connectionId: "c-prod",
      connectionName: "lumen-api",
      envColor: "env-production",
      environmentName: "Production",
    },
  )
  auditRow(
    2 * HOUR,
    "upsertScheduler",
    {
      kind: "scheduler",
      data: { queueId: "bull:emails", queueName: "emails", schedulerKey: "daily-report" },
    },
    { outcomeReason: "send-daily-report · 0 9 * * * (Europe/Brussels)" },
  )
  auditRow(
    3 * HOUR,
    "drainQueue",
    { kind: "queue", data: { queueId: "bull:images", queueName: "images" } },
    { outcome: "failure", outcomeReason: "0 removed; stopped on error: Connection reset by peer" },
  )
  auditRow(5 * HOUR, "pauseQueue", {
    kind: "queue",
    data: { queueId: "bull:images", queueName: "images" },
  })
  auditRow(
    DAY,
    "retryJob",
    // A row written before targets carried the queue's id.
    { kind: "job", data: { queueId: null, queueName: "emails", jobId: "3120" } },
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
 * `?preview=empty` opens Ranch as on a first launch (no group, no
 * connection, the first-run wizard) and `?preview=full` brings the demo
 * world back. Remembered for the tab's session, so it survives reloads.
 */
export const SCENARIO_KEY = "ranchPreview.scenario"

export function readScenario(): "full" | "empty" {
  try {
    const asked = new URLSearchParams(window.location.search).get("preview")
    if (asked === "empty" || asked === "full") sessionStorage.setItem(SCENARIO_KEY, asked)
    return sessionStorage.getItem(SCENARIO_KEY) === "empty" ? "empty" : "full"
  } catch {
    return "full"
  }
}

export const previewDismissed = new Set<string>()

export function demoNote(conn: Connection, signature: string, dismissed: boolean) {
  return {
    id: "note-1",
    connectionId: conn.id,
    connectionName: conn.name,
    queueId: "bull:emails",
    queueName: "emails",
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
export function aiOffAsked(): boolean {
  try {
    return new URLSearchParams(window.location.search).get("ai") === "off"
  } catch {
    return false
  }
}

export function buildScenario(): World {
  const world = buildScenarioWorld()
  if (aiOffAsked()) world.kv.set("ai_enabled", "false")
  if (manyQueuesAsked()) addManyQueues(world)
  return world
}

/** `?queues=many`: 42 queues on c-staging, for the Map. */
export function manyQueuesAsked(): boolean {
  try {
    return new URLSearchParams(window.location.search).get("queues") === "many"
  } catch {
    return false
  }
}

export function addManyQueues(world: World) {
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

export function buildScenarioWorld(): World {
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
