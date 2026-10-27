import { useMutation, useQueryClient } from "@tanstack/react-query"

import { type BulkActionResult, type BulkFailure, commands } from "@/lib/tauri/bindings"
import type { JobSelectionEntry } from "@/stores/useJobSelectionStore"

/**
 * Frontend orchestrator hooks for cross-queue bulk
 * actions (K > 1). Each hook fans out one single-queue
 * `bulkXxxJobs` Tauri call per queue in parallel via
 * `Promise.allSettled` and aggregates the per-queue results.
 *
 * Architecture notes:
 *   - No new Tauri command. Parity stays 30.
 *   - The per-connection actor serializes the K parallel calls on
 *     its mailbox — concurrency-on-the-wire = K, execution-
 *     serialization = 1.
 *   - K audit rows land in the audit log (one per queue), within
 *     milliseconds of each other. The user's "I did one cross-queue
 *     thing" lives in the BulkActionBar's per-queue summary.
 *   - `connectionLocked` rejections are collected into
 *     `PerQueueOutcome.error` so the BulkActionBar can consolidate
 *     the locked message in the completion summary.
 */

export type BulkActionKind = "retry" | "remove" | "promote"

export interface CrossQueueBulkInput {
  connectionId: string
  queueGroups: Map<string, JobSelectionEntry[]>
}

export interface PerQueueOutcome {
  queueName: string
  /** Number of jobs the user selected in this queue. */
  total: number
  /** Number that completed successfully (per the per-queue actor). */
  succeeded: number
  /** Per-job failures collected by the per-queue actor (state
   *  precondition mismatch, individual Redis errors, etc.). */
  failed: BulkFailure[]
  cancelled: boolean
  /** Set when the per-queue Tauri call itself errored (e.g.
   *  connection locked, network glitch) — distinct from per-job
   *  failures which live in `failed`. The BulkActionBar surfaces
   *  this to the summary so the user knows the queue was blocked. */
  error?: string
}

export interface CrossQueueBulkResult {
  action: BulkActionKind
  /** Aggregate total across all queues. */
  total: number
  /** Aggregate successes across all queues. */
  succeeded: number
  /** Aggregate per-job failures across all queues. */
  failed: BulkFailure[]
  /** True when any per-queue call returned `cancelled = true`. */
  cancelled: boolean
  /** Per-queue breakdown for the completion summary. */
  perQueue: PerQueueOutcome[]
}

/** Aggregate a `Promise.allSettled` array of per-queue results
 *  into a single `CrossQueueBulkResult`. Per-queue Tauri-call
 *  rejections become `PerQueueOutcome.error` entries; per-job
 *  failures (collected by the per-queue actor) accumulate into
 *  the aggregate `failed` array. */
export function aggregateCrossQueueResults(
  action: BulkActionKind,
  queueGroups: Map<string, JobSelectionEntry[]>,
  settled: Array<PromiseSettledResult<{ queueName: string; result: BulkActionResult }>>,
  /** Pre-sorted list of queue names matching the settled order so
   *  rejected entries can be attributed to the right queue (a
   *  `PromiseSettledResult` carries no key of its own). */
  orderedQueueNames: string[],
): CrossQueueBulkResult {
  const perQueue: PerQueueOutcome[] = []
  let total = 0
  let succeeded = 0
  let cancelled = false
  const failed: BulkFailure[] = []

  for (let i = 0; i < settled.length; i++) {
    const queueName = orderedQueueNames[i]
    const r = settled[i]
    if (!queueName || !r) continue
    const entries = queueGroups.get(queueName) ?? []
    if (r.status === "fulfilled") {
      const pq: PerQueueOutcome = {
        queueName,
        total: r.value.result.total,
        succeeded: r.value.result.succeeded,
        failed: r.value.result.failed,
        cancelled: r.value.result.cancelled,
      }
      perQueue.push(pq)
      total += r.value.result.total
      succeeded += r.value.result.succeeded
      failed.push(...r.value.result.failed)
      if (r.value.result.cancelled) cancelled = true
    } else {
      const reason = r.reason instanceof Error ? r.reason.message : String(r.reason)
      perQueue.push({
        queueName,
        total: entries.length,
        succeeded: 0,
        failed: [],
        cancelled: false,
        error: reason,
      })
      total += entries.length
      // Per-queue Tauri-call rejections don't add to per-job
      // failures — they live in `error` for the summary view.
    }
  }

  return { action, total, succeeded, failed, cancelled, perQueue }
}

function invalidateAfterCrossQueueBulk(
  qc: ReturnType<typeof useQueryClient>,
  input: CrossQueueBulkInput,
) {
  for (const [queueName, entries] of input.queueGroups.entries()) {
    const prefix = entries[0]?.jobRef.queue.prefix ?? "bull"
    qc.invalidateQueries({
      queryKey: ["jobs", input.connectionId, prefix, queueName],
    })
    for (const e of entries) {
      qc.invalidateQueries({
        queryKey: ["job", input.connectionId, prefix, queueName, e.jobRef.id],
      })
    }
  }
  qc.invalidateQueries({ queryKey: ["queues", input.connectionId] })
}

/** Shared dispatcher for the 3 cross-queue mutation hooks. The
 *  per-action wrapper picks the matching `commands.bulkXxxJobs`. */
async function dispatchCrossQueue(
  action: BulkActionKind,
  input: CrossQueueBulkInput,
): Promise<CrossQueueBulkResult> {
  const fn =
    action === "retry"
      ? commands.bulkRetryJobs
      : action === "remove"
        ? commands.bulkRemoveJobs
        : commands.bulkPromoteJobs

  // Stable order so we can attribute `PromiseSettledResult` entries
  // back to the right queue name when one rejects.
  const orderedQueueNames = Array.from(input.queueGroups.keys())

  const settled = await Promise.allSettled(
    orderedQueueNames.map(async (queueName) => {
      const entries = input.queueGroups.get(queueName) ?? []
      const queueRef = entries[0]?.jobRef.queue
      if (!queueRef) {
        // Defensive — should never happen since groupByQueue only
        // produces entries with at least one item.
        throw new Error(`No queue ref for ${queueName}`)
      }
      const r = await fn({
        connectionId: input.connectionId,
        queueRef,
        jobIds: entries.map((e) => e.jobRef.id),
      })
      if (r.status === "error") {
        const err = new Error(r.error.message)
        ;(err as Error & { kind?: string }).kind = r.error.kind
        throw err
      }
      return { queueName, result: r.data }
    }),
  )

  return aggregateCrossQueueResults(action, input.queueGroups, settled, orderedQueueNames)
}

export function useBulkRetryCrossQueue() {
  const qc = useQueryClient()
  return useMutation<CrossQueueBulkResult, Error, CrossQueueBulkInput>({
    mutationFn: (input) => dispatchCrossQueue("retry", input),
    onSettled: (_d, _e, input) => invalidateAfterCrossQueueBulk(qc, input),
  })
}

export function useBulkRemoveCrossQueue() {
  const qc = useQueryClient()
  return useMutation<CrossQueueBulkResult, Error, CrossQueueBulkInput>({
    mutationFn: (input) => dispatchCrossQueue("remove", input),
    onSettled: (_d, _e, input) => invalidateAfterCrossQueueBulk(qc, input),
  })
}

export function useBulkPromoteCrossQueue() {
  const qc = useQueryClient()
  return useMutation<CrossQueueBulkResult, Error, CrossQueueBulkInput>({
    mutationFn: (input) => dispatchCrossQueue("promote", input),
    onSettled: (_d, _e, input) => invalidateAfterCrossQueueBulk(qc, input),
  })
}
