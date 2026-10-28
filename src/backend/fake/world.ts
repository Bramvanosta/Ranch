/** The live demo world, and the helpers the fake commands share over it. */
import type {
  AuditLogEntry,
  Connection,
  FlowNode,
  Job,
  JobState,
  Queue,
  QueueId,
  QueueMetrics,
} from "@/lib/tauri/bindings"
import { between, buildScenario } from "./seed"
import { type Args, DAY, HOUR, MIN, PreviewError, type QueueWorld } from "./types"

export let world = buildScenario()

/** A fresh demo world (`ranchPreview.reset()`). */
export function resetWorld() {
  world = buildScenario()
}

/** The version `?update=0.3.0` on the first URL offers, to review the update flow. */
export let offeredUpdate: string | null = null

export function offerUpdate(version: string | null) {
  offeredUpdate = version
}

// ── helpers over the world ───────────────────────────────────────────────
export const UNREACHABLE = "c-side"

export function connectionOf(id: unknown): Connection {
  const c = world.connections.find((x) => x.id === id)
  if (!c) throw new PreviewError("connectionNotFound", `Connection not found: ${String(id)}`)
  return c
}

export function queuesOf(connectionId: unknown): QueueWorld[] {
  connectionOf(connectionId)
  if (connectionId === UNREACHABLE) {
    throw new PreviewError("redis", "Connection refused (os error 61)")
  }
  return world.queues.get(String(connectionId)) ?? []
}

export function queueOf(connectionId: unknown, id: unknown): QueueWorld {
  const q = queuesOf(connectionId).find((x) => x.id === id)
  if (!q)
    throw new PreviewError(
      "invalidInput",
      `Queue "${String(id)}" does not exist on this connection`,
    )
  return q
}

export function isLocked(connectionId: string): boolean {
  return (
    connectionOf(connectionId).envColor === "env-production" && !world.unlocked.has(connectionId)
  )
}

export function guardWrite(connectionId: unknown) {
  const id = String(connectionId)
  if (isLocked(id)) {
    throw new PreviewError(
      "connectionLocked",
      "This production connection is locked. Unlock it for the session (cmd-shift-U) to make changes.",
    )
  }
}

export function len(q: QueueWorld, state: JobState): number {
  return q.jobs.get(state)?.length ?? 0
}

/** The states a BullMQ queue counts, as the backend reads them. */
const COUNTED: JobState[] = [
  "waiting",
  "active",
  "completed",
  "failed",
  "delayed",
  "prioritized",
  "paused",
]

export function toQueue(q: QueueWorld): Queue {
  return {
    id: q.id,
    name: q.name,
    isPaused: q.paused,
    counts: Object.fromEntries(COUNTED.map((state) => [state, len(q, state)])),
  } as Queue
}

export function findJob(q: QueueWorld, id: string): { job: Job; state: JobState } | null {
  for (const [state, list] of q.jobs) {
    const job = list.find((j) => j.jobRef.id === id)
    if (job) return { job, state }
  }
  return null
}

export function moveJob(
  q: QueueWorld,
  id: string,
  from: JobState,
  to: JobState,
  patch: Partial<Job>,
) {
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

export function audit(
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

export const RANGE_MS: Record<string, number> = {
  "5m": 5 * MIN,
  "15m": 15 * MIN,
  "30m": 30 * MIN,
  "1h": HOUR,
  "6h": 6 * HOUR,
  "24h": DAY,
  "3d": 3 * DAY,
  "7d": 7 * DAY,
}

export function metrics(queues: QueueWorld[], range: string): QueueMetrics {
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
            const key = `${q.name}/${job.name}`
            const entry = byName.get(key) ?? { queueName: q.name, name: job.name, took: [] }
            entry.took.push(took)
            byName.set(key, entry)
            slow.push({
              queueId: q.id,
              queueName: q.name,
              jobId: job.jobRef.id,
              name: job.name,
              processingTimeMs: took,
              finishedOn: job.finishedOn,
            })
          }
        }
        if (state === "failed") {
          const key = `${q.name}/${job.name}`
          const entry = types.get(key) ?? {
            queueId: q.id,
            queueName: q.name,
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

export function nextRuns(schedule: Args): number[] {
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
      `A cron pattern has 5 fields (minute hour day month weekday) or 6 with seconds first; this one has ${fields.length}`,
    )
  }
  if (/[LW#?@]/.test(pattern)) {
    throw new PreviewError(
      "invalidInput",
      "'L' is not supported in a cron pattern here; use numbers, * , - / and month or weekday names",
    )
  }
  // Preview only: not a cron engine: the next five days at the pattern's hour.
  const hour = Number(fields[fields.length === 6 ? 2 : 1])
  const minute = Number(fields[fields.length === 6 ? 1 : 0])
  const base = new Date(now)
  base.setHours(Number.isFinite(hour) ? hour : 9, Number.isFinite(minute) ? minute : 0, 0, 0)
  return [1, 2, 3, 4, 5].map((d) => base.getTime() + d * DAY)
}

export function flowFor(connectionId: unknown, queueId: QueueId, jobId: string) {
  const local = queuesOf(connectionId)
  const orders = local.find((q) => q.name === "orders")
  const parent = orders?.jobs.get("waitingChildren")?.[0]
  const emails = local.find((q) => q.name === "emails")
  const children = [
    ...(emails?.jobs.get("completed") ?? []),
    ...(emails?.jobs.get("failed") ?? []),
  ].filter((j) => j.parentKey === "bull:orders:flow-1")
  // A demo queue's id is its key base, as with BullMQ.
  const focusKey = `${queueId}:${jobId}`
  const node = (job: Job, parentKey: string | null): FlowNode => ({
    key: `${job.jobRef.queueId}:${job.jobRef.id}`,
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
  const queue = queueOf(connectionId, queueId)
  const found = findJob(queue, jobId)
  if (!found) throw new PreviewError("jobNotFound", `Job not found: ${queue.name}/${jobId}`)
  return {
    nodes: [node(found.job, null)],
    rootKey: focusKey,
    focusKey,
    truncated: false,
    blocked: false,
  }
}

export function requestApproval(tool = "bulk_remove_jobs") {
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
      queueId: "bull:emails",
    }),
    targetPreviewJson: JSON.stringify(previews[tool] ?? null),
    timeoutSeconds: 120,
  })
}

export const a = (args: Args) => (args.args ?? {}) as Args

/** The web preview's failure signature (see `list_failure_causes`). */
export function previewSignature(reason: string): string {
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
