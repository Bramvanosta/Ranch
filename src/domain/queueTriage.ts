import { countOf, inLine } from "@/domain/stateCounts"
import type { JobStateInfo, Queue } from "@/lib/tauri/bindings"

/**
 * Which queues of a connection need the user.
 *
 * - `failed`: jobs failed after their last attempt;
 * - `paused`: the queue does not hand out jobs;
 * - `noWorker`: jobs are waiting and no worker is attached (only when the
 *   worker count is known: a Redis that refuses `CLIENT LIST` never marks one).
 *
 * Problems come first, worst kind first, then by how many jobs they concern;
 * every other queue folds into "+ N more queues", by name. The jobs waiting
 * are those in the states the backend describes as waiting (`inLine`).
 */
export type QueueProblem = "failed" | "paused" | "noWorker"

export interface TriagedQueue {
  queue: Queue
  problem: QueueProblem
  /** The number the row shows: failed jobs, or jobs waiting. */
  count: number
}

const SEVERITY: Record<QueueProblem, number> = { failed: 0, paused: 1, noWorker: 2 }

const byName = (a: Queue, b: Queue) => a.name.localeCompare(b.name)

/** Worker counts keyed by queue id; `undefined` = unknown. */
export type WorkerCounts = ReadonlyMap<string, number> | undefined

export function problemOf(
  queue: Queue,
  workers: WorkerCounts,
  states: ReadonlyArray<JobStateInfo>,
): Omit<TriagedQueue, "queue"> | null {
  const failed = countOf(queue.counts, "failed")
  if (failed > 0) return { problem: "failed", count: failed }
  const waiting = inLine(queue.counts, states)
  if (queue.isPaused) return { problem: "paused", count: waiting }
  const attached = workers?.get(queue.id)
  if (attached === 0 && waiting > 0) return { problem: "noWorker", count: waiting }
  return null
}

export function triageQueues(
  queues: readonly Queue[],
  workers: WorkerCounts,
  states: ReadonlyArray<JobStateInfo>,
): { problems: TriagedQueue[]; rest: Queue[] } {
  const problems: TriagedQueue[] = []
  const rest: Queue[] = []
  for (const queue of queues) {
    const p = problemOf(queue, workers, states)
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
  return (queues ?? []).reduce((sum, q) => sum + countOf(q.counts, "failed"), 0)
}
