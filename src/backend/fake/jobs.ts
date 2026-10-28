import type { AuditLogEntry, Job, JobRef, JobState } from "@/lib/tauri/bindings"
import type { FakeHandlers } from "./handlers"
import { makeJob } from "./seed"
import { type Args, PreviewError, type QueueWorld } from "./types"
import {
  a,
  audit,
  findJob,
  guardWrite,
  len,
  metrics,
  moveJob,
  previewSignature,
  queueOf,
  queuesOf,
  toQueue,
} from "./world"

/** Queues and their jobs: lists, failures by cause, single and bulk actions, pause. */
export const jobsHandlers = {
  // queues & jobs
  list_queues: (args) => queuesOf(a(args).connectionId).map(toQueue),
  list_jobs: (args) => {
    const input = a(args)
    const q = queueOf(input.connectionId, input.queueId)
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
    const queue = queueOf(input.connectionId, input.queueId)
    const found = findJob(queue, String(input.jobId))
    if (!found)
      throw new PreviewError("jobNotFound", `Job not found: ${queue.name}/${String(input.jobId)}`)
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
            "SMTP: 250 2.0.0 OK, queued as 4F2A91",
          ]
    return { lines, total: lines.length, hasMore: false }
  },
  list_recent_jobs: (args) => {
    const input = a(args)
    const scope = input.queueId
      ? [queueOf(input.connectionId, input.queueId)]
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
    if (input.queue) jobs = jobs.filter((j) => j.jobRef.queueName === input.queue)
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
        .map((q) => ({ queueId: q.id, queueName: q.name, failed: len(q, "failed") })),
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
    const all = queuesOf(input.connectionId).filter((q) => !input.queue || q.name === input.queue)
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
        jobs: JobRef[]
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
      c.queues.set(j.jobRef.queueName, (c.queues.get(j.jobRef.queueName) ?? 0) + 1)
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
    const q = queueOf(input.connectionId, input.queueId)
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
    const job = makeJob(q, id, spec.name, state, Date.now())
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
      { kind: "job", data: { queueId: q.id, queueName: q.name, jobId: id } },
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
    const q = queueOf(input.connectionId, input.queueId)
    const found = findJob(q, String(input.jobId))
    if (!found)
      throw new PreviewError("jobNotFound", `Job not found: ${q.name}/${String(input.jobId)}`)
    q.jobs.set(
      found.state,
      (q.jobs.get(found.state) ?? []).filter((j) => j !== found.job),
    )
    audit(String(input.connectionId), "removeJob", {
      kind: "job",
      data: { queueId: q.id, queueName: q.name, jobId: String(input.jobId) },
    })
    return { jobId: String(input.jobId), newState: null, action: "remove" }
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
} satisfies FakeHandlers

function jobAction(
  input: Args,
  from: JobState,
  to: JobState,
  action: string,
  auditAction: AuditLogEntry["action"],
  patch: Partial<Job>,
) {
  guardWrite(input.connectionId)
  const q = queueOf(input.connectionId, input.queueId)
  const target = to === "waiting" && q.paused ? "paused" : to
  if (!moveJob(q, String(input.jobId), from, target, patch)) {
    throw new PreviewError("invalidJobState", `Job ${String(input.jobId)} is not ${from}`)
  }
  audit(String(input.connectionId), auditAction, {
    kind: "job",
    data: { queueId: q.id, queueName: q.name, jobId: String(input.jobId) },
  })
  return { jobId: String(input.jobId), newState: target, action }
}

function bulk(input: Args, action: string, apply: (q: QueueWorld, id: string) => boolean) {
  guardWrite(input.connectionId)
  const q = queueOf(input.connectionId, input.queueId)
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
  const q = queueOf(input.connectionId, input.queueId)
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
    data: { queueId: q.id, queueName: q.name },
  })
  return { isPaused: paused, alreadyInState: already }
}
