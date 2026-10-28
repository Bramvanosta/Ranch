import { useMutation, useQueryClient } from "@tanstack/react-query"
import { queueKeys } from "@/data/queues/keys"
import { type CommandResult, unwrap } from "@/data/result"
import {
  type BulkActionArgs,
  type BulkActionResult,
  type BulkFailure,
  commands,
  type QueueId,
} from "@/lib/tauri/bindings"
import type { JobSelectionEntry } from "@/stores/useJobSelectionStore"
import { jobKeys } from "./keys"

/**
 * Frontend orchestrator hooks for cross-queue bulk
 * actions (K > 1). Each hook fans out one single-queue
 * `bulkXxxJobs` Tauri call per queue in parallel via
 * `Promise.allSettled` and aggregates the per-queue results.
 *
 * Architecture notes:
 *   - No new Tauri command. Parity stays 30.
 *   - The per-connection actor serializes the K parallel calls on
 *     its mailbox: concurrency-on-the-wire = K, execution-
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
  /** The selected jobs, by queue id. */
  queueGroups: Map<QueueId, JobSelectionEntry[]>
}

export interface PerQueueOutcome {
  queueId: QueueId
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
   *  connection locked, network glitch), distinct from per-job
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
  queueGroups: Map<QueueId, JobSelectionEntry[]>,
  settled: Array<PromiseSettledResult<{ queueId: QueueId; result: BulkActionResult }>>,
  /** Pre-sorted list of queue ids matching the settled order so
   *  rejected entries can be attributed to the right queue (a
   *  `PromiseSettledResult` carries no key of its own). */
  orderedQueueIds: QueueId[],
): CrossQueueBulkResult {
  const perQueue: PerQueueOutcome[] = []
  let total = 0
  let succeeded = 0
  let cancelled = false
  const failed: BulkFailure[] = []

  for (let i = 0; i < settled.length; i++) {
    const queueId = orderedQueueIds[i]
    const r = settled[i]
    if (queueId === undefined || !r) continue
    const entries = queueGroups.get(queueId) ?? []
    const queueName = entries[0]?.jobRef.queueName ?? queueId
    if (r.status === "fulfilled") {
      const pq: PerQueueOutcome = {
        queueId,
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
        queueId,
        queueName,
        total: entries.length,
        succeeded: 0,
        failed: [],
        cancelled: false,
        error: reason,
      })
      total += entries.length
      // Per-queue Tauri-call rejections don't add to per-job
      // failures; they live in `error` for the summary view.
    }
  }

  return { action, total, succeeded, failed, cancelled, perQueue }
}

function invalidateAfterCrossQueueBulk(
  qc: ReturnType<typeof useQueryClient>,
  input: CrossQueueBulkInput,
) {
  for (const [queueId, entries] of input.queueGroups.entries()) {
    qc.invalidateQueries({ queryKey: jobKeys.lists(input.connectionId, queueId) })
    for (const e of entries) {
      qc.invalidateQueries({ queryKey: jobKeys.detail(input.connectionId, queueId, e.jobRef.id) })
    }
  }
  qc.invalidateQueries({ queryKey: queueKeys.list(input.connectionId) })
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
  // back to the right queue when one rejects.
  const orderedQueueIds = Array.from(input.queueGroups.keys())

  const settled = await Promise.allSettled(
    orderedQueueIds.map(async (queueId) => {
      const entries = input.queueGroups.get(queueId) ?? []
      const r = unwrap(
        await fn({
          connectionId: input.connectionId,
          queueId,
          jobIds: entries.map((e) => e.jobRef.id),
        }),
      )
      return { queueId, result: r }
    }),
  )

  return aggregateCrossQueueResults(action, input.queueGroups, settled, orderedQueueIds)
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

/**
 * TanStack Query mutation hooks for the 3 bulk actions.
 *
 * Each hook calls the matching `commands.bulkXxxJobs` Tauri command,
 * which dispatches via the connection actor (ONE lock check, ONE
 * consolidated audit row). The actor emits `bulk:progress` events
 * during the loop; the global event listener (mounted in App.tsx)
 * feeds those into `useBulkProgressStore` so the UI updates live.
 *
 * On success: invalidates `jobKeys.all(...)` and `queueKeys.list(...)` so
 * the next poll picks up the post-bulk state. No optimistic per-row
 * flips: the BulkActionBar shows the consolidated summary; per-row
 * visuals come from the progress store.
 *
 * On error (notably `connectionLocked`): the error's `kind` is
 * kept on the thrown `RanchError`
 * so the `BulkActionBar` can synthesize a "consolidated locked
 * summary" view.
 */
export interface BulkActionInput {
  connectionId: string
  queueId: QueueId
  jobIds: string[]
}

function invalidateAfterBulk(qc: ReturnType<typeof useQueryClient>, input: BulkActionInput) {
  qc.invalidateQueries({
    queryKey: jobKeys.lists(input.connectionId, input.queueId),
  })
  qc.invalidateQueries({ queryKey: queueKeys.list(input.connectionId) })
  // Invalidate any cached single-job entries for the affected job ids.
  for (const jobId of input.jobIds) {
    qc.invalidateQueries({
      queryKey: jobKeys.detail(input.connectionId, input.queueId, jobId),
    })
  }
}

export function useBulkRetry() {
  const qc = useQueryClient()
  return useMutation<BulkActionResult, Error, BulkActionInput>({
    mutationFn: async (input) => {
      return unwrap(
        await commands.bulkRetryJobs({
          connectionId: input.connectionId,
          queueId: input.queueId,
          jobIds: input.jobIds,
        }),
      )
    },
    onSettled: (_d, _e, input) => invalidateAfterBulk(qc, input),
  })
}

export function useBulkRemove() {
  const qc = useQueryClient()
  return useMutation<BulkActionResult, Error, BulkActionInput>({
    mutationFn: async (input) => {
      return unwrap(
        await commands.bulkRemoveJobs({
          connectionId: input.connectionId,
          queueId: input.queueId,
          jobIds: input.jobIds,
        }),
      )
    },
    onSettled: (_d, _e, input) => invalidateAfterBulk(qc, input),
  })
}

export function useBulkPromote() {
  const qc = useQueryClient()
  return useMutation<BulkActionResult, Error, BulkActionInput>({
    mutationFn: async (input) => {
      return unwrap(
        await commands.bulkPromoteJobs({
          connectionId: input.connectionId,
          queueId: input.queueId,
          jobIds: input.jobIds,
        }),
      )
    },
    onSettled: (_d, _e, input) => invalidateAfterBulk(qc, input),
  })
}

/** `bulk_retry_jobs` as a plain call, for a flow that runs it under a one-shot unlock. */
export function bulkRetryJobs(args: BulkActionArgs): Promise<CommandResult<BulkActionResult>> {
  return commands.bulkRetryJobs(args)
}
