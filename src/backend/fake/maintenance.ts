import type { JobState } from "@/lib/tauri/bindings"
import type { FakeHandlers } from "./handlers"
import { a, audit, guardWrite, len, queueOf } from "./world"

/** Drain, clean and retry-all, with their previews. */
export const maintenanceHandlers = {
  // maintenance
  preview_queue_maintenance: (args) => {
    const input = a(args)
    const q = queueOf(input.connectionId, input.queueId)
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
    const q = queueOf(input.connectionId, input.queueId)
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
      { kind: "queue", data: { queueId: q.id, queueName: q.name } },
      `${affected} removed`,
    )
    return { action: "drain", affected, skipped: 0, cancelled: false }
  },
  clean_queue: (args) => {
    const input = a(args)
    guardWrite(input.connectionId)
    const q = queueOf(input.connectionId, input.queueId)
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
      { kind: "queue", data: { queueId: q.id, queueName: q.name } },
      `${affected} removed`,
    )
    return { action: "clean", affected, skipped: 0, cancelled: false }
  },
  retry_all_failed: (args) => {
    const input = a(args)
    guardWrite(input.connectionId)
    const q = queueOf(input.connectionId, input.queueId)
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
      { kind: "queue", data: { queueId: q.id, queueName: q.name } },
      `${failed.length} retried`,
    )
    return { action: "retry", affected: failed.length, skipped: 0, cancelled: false }
  },
} satisfies FakeHandlers
