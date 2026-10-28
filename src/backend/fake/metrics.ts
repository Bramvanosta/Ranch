import type { AttentionItem, JobRef, MetricsRange, QueueId, Throughput } from "@/lib/tauri/bindings"
import type { FakeHandlers, InvokeArgs } from "./handlers"
import { jobsHandlers } from "./jobs"
import { DAY } from "./types"
import { a, flowFor, len, metrics, queueOf, queuesOf, RANGE_MS, UNREACHABLE, world } from "./world"

/** Workers, flows, metrics, attention, live events, the Map and the Timeline. */
export const metricsHandlers = {
  // workers, flows, metrics
  count_queue_workers: (args) => {
    const input = a(args)
    // Production forbids CLIENT LIST, as in `list_workers` below.
    if (input.connectionId === "c-prod") return { supported: false, counts: [] }
    const refs = (input.queueIds ?? []) as QueueId[]
    return {
      supported: true,
      counts: refs.map((queueId) => ({
        queueId,
        workers: queueOf(input.connectionId, queueId).workers.length,
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
      workers: queueOf(input.connectionId, input.queueId).workers,
    }
  },
  get_flow: (args) =>
    flowFor(a(args).connectionId, a(args).queueId as QueueId, String(a(args).jobId)),
  list_flow_roots: (args) => {
    const q = queueOf(a(args).connectionId, a(args).queueId)
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
    metrics([queueOf(a(args).connectionId, a(args).queueId)], String(a(args).range ?? "24h")),
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
        queueId: "bull:emails",
        jobIds: ["1081", "1082", "1083"],
      }),
    }
  },
  // What needs the user across connections, from the same
  // world as the other stubs: causes, paused / worker-less queues, offline.
  list_attention: () => {
    const items: AttentionItem[] = []
    for (const c of world.connections) {
      const base = {
        connectionId: c.id,
        connectionName: c.name,
        envColor: c.envColor,
        envLabel: c.envLabel,
        queueId: null as string | null,
        queueName: null as string | null,
        sample: null as string | null,
        sinceMs: null as number | null,
        trend: [] as number[],
        jobs: [] as JobRef[],
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
      const causes = jobsHandlers.list_failure_causes({
        args: { connectionId: c.id, queue: null },
      }) as {
        causes: Array<{
          signature: string
          sample: string
          count: number
          firstAt: number
          queueNames: string[]
          trend: number[]
          jobs: JobRef[]
        }>
      }
      for (const cause of causes.causes.slice(0, 10)) {
        items.push({
          ...base,
          key: `failing:${c.id}:${cause.signature}`,
          kind: "failing",
          queueId: cause.queueNames.length === 1 ? (cause.jobs[0]?.queueId ?? null) : null,
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
            key: `paused:${c.id}:${q.name}`,
            kind: "stuck",
            queueId: q.id,
            queueName: q.name,
            reason: "paused",
            count: waiting,
          })
        } else if (waiting > 0 && c.id !== "c-prod" && q.workers.length === 0) {
          items.push({
            ...base,
            key: `noworker:${c.id}:${q.name}`,
            kind: "stuck",
            queueId: q.id,
            queueName: q.name,
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
    const cursors = (input.cursors ?? []) as Array<{ queueId: string; lastId: string }>
    const events = queuesOf(input.connectionId).flatMap((q) =>
      (["completed", "failed"] as const).flatMap((state) =>
        (q.jobs.get(state) ?? [])
          .filter((j) => j.finishedOn)
          .map((j) => ({
            queueId: q.id,
            queueName: q.name,
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
        queueId: q.id,
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
  get_throughput: (args) => throughputOf(args),

  // The Map: flow links from the demo jobs' parent keys.
  // No event streams in the fake world: the preview polls.
  follow_connection_events: () => false,
  get_topology: (args) => {
    const queues = queuesOf(a(args).connectionId)
    const links = new Map<string, { parent: QueueId; child: QueueId; jobs: number }>()
    for (const q of queues) {
      for (const jobs of q.jobs.values()) {
        for (const job of jobs) {
          // A demo job key is `{queue id}:{job id}`, as with BullMQ.
          const at = job.parentKey?.lastIndexOf(":") ?? -1
          const parent = at > 0 ? job.parentKey?.slice(0, at) : undefined
          if (!parent) continue
          const key = `${parent}>${q.id}`
          const link = links.get(key) ?? {
            parent,
            child: q.id,
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
          queueId: q.id,
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
    const t = throughputOf({
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
      jobsHandlers.list_failure_causes({
        args: { connectionId: input.connectionId, queue: null },
      }) as { causes: Array<{ signature: string; sample: string; jobs: JobRef[] }> } | undefined
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
} satisfies FakeHandlers

/** Per-queue series from the finished jobs; `reports` records no metrics. */
function throughputOf(args: InvokeArgs): Throughput {
  const input = a(args)
  const range = String(input.range ?? "24h") as MetricsRange
  const span = RANGE_MS[range] ?? DAY
  const count = range === "1h" ? 12 : range === "6h" ? 12 : 24
  const bucketMs = Math.ceil(span / count)
  const now = Date.now()
  const start = now - span
  const completed = new Array<number>(count).fill(0)
  const failed = new Array<number>(count).fill(0)
  const queues = queuesOf(input.connectionId).map((q) => {
    if (q.name === "reports") {
      return { queueId: q.id, queueName: q.name, metricsOff: true, completed: [], failed: [] }
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
    return { queueId: q.id, queueName: q.name, metricsOff: false, completed: c, failed: f }
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
}
