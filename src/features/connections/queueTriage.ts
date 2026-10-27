import type { Queue } from "@/lib/tauri/bindings"

/**
 * Which queues of a connection need the user.
 *
 * - `failed`: jobs failed after their last attempt;
 * - `paused`: the queue does not hand out jobs;
 * - `noWorker`: jobs are waiting and no worker is attached (only when the
 *   worker count is known: a Redis that refuses `CLIENT LIST` never marks one).
 *
 * Problems come first, worst kind first, then by how many jobs they concern;
 * every other queue folds into "+ N more queues", by name.
 */
export type QueueProblem = "failed" | "paused" | "noWorker"

export interface TriagedQueue {
  queue: Queue
  problem: QueueProblem
  /** The number the row shows: failed jobs, or jobs waiting. */
  count: number
}

const SEVERITY: Record<QueueProblem, number> = { failed: 0, paused: 1, noWorker: 2 }

const byName = (a: Queue, b: Queue) => a.queueRef.name.localeCompare(b.queueRef.name)

const waitingOf = (q: Queue) => Number(q.counters.waiting) + Number(q.counters.prioritized ?? 0)

/** Worker counts keyed by queue name; `undefined` = unknown. */
export type WorkerCounts = ReadonlyMap<string, number> | undefined

export function problemOf(queue: Queue, workers: WorkerCounts): Omit<TriagedQueue, "queue"> | null {
  const failed = Number(queue.counters.failed)
  if (failed > 0) return { problem: "failed", count: failed }
  if (queue.isPaused) return { problem: "paused", count: waitingOf(queue) }
  const attached = workers?.get(queue.queueRef.name)
  if (attached === 0 && waitingOf(queue) > 0)
    return { problem: "noWorker", count: waitingOf(queue) }
  return null
}

export function triageQueues(
  queues: readonly Queue[],
  workers: WorkerCounts,
): { problems: TriagedQueue[]; rest: Queue[] } {
  const problems: TriagedQueue[] = []
  const rest: Queue[] = []
  for (const queue of queues) {
    const p = problemOf(queue, workers)
    if (p) problems.push({ queue, ...p })
    else rest.push(queue)
  }
  problems.sort(
    (a, b) =>
      SEVERITY[a.problem] - SEVERITY[b.problem] || b.count - a.count || byName(a.queue, b.queue),
  )
  rest.sort(byName)
  return { problems, rest }
}

/** Failed jobs across a connection's queues (the number a folded row keeps). */
export function failedTotal(queues: readonly Queue[] | undefined): number {
  return (queues ?? []).reduce((sum, q) => sum + Number(q.counters.failed), 0)
}
