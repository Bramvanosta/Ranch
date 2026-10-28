import { create } from "zustand"

/**
 * Tracks the currently-running bulk action(s) so
 * the `BulkActionBar` and `JobListRow`s can render live per-row visuals.
 *
 * Progress is kept per queue, as `Map<queueId, …>` rather than scalar
 * `completed / failedSoFar / runningAction`, to support concurrent cross-queue
 * bulks. Aggregate getters compose the per-queue values for the
 * BulkActionBar's summary view; K=1 callers see the same shape via
 * the aggregate paths (a single-entry map sums to itself).
 *
 * The store is fed by:
 *   - `BulkActionBar` calling `start()` when the user presses Enter
 *     to execute (initializes the per-queue maps in one call).
 *   - A global Tauri event listener (`listen("bulk:progress", …)`)
 *     mounted at the app root, which calls `tick(queueId, …)` per
 *     chunk emitted by the connection actor, disambiguated by
 *     `queueId` so concurrent per-queue progress streams don't
 *     stomp on each other.
 *   - `BulkActionBar` calling `finish()` when the mutation resolves
 *     (or the user cancels); it clears all maps in one shot.
 */
export type BulkActionKind = "retry" | "remove" | "promote"
export type PerRowState = "queued" | "executing" | "succeeded" | "failed"

export interface RunningAction {
  action: BulkActionKind
  total: number
}

interface BulkProgressStore {
  /** Per-queue `{ action, total }` for every queue currently in
   *  flight. Empty when no bulk is running. */
  runningActions: Map<string, RunningAction>
  /** Per-queue completed count. Updated by `tick()`. */
  perQueueCompleted: Map<string, number>
  /** Per-queue failed count. Updated by `tick()`. */
  perQueueFailedSoFar: Map<string, number>
  /** Per-row visual state for rows currently in any bulk run. */
  perRow: Map<string, PerRowState>

  /** Initialize per-queue maps from a `Map<queueId, {action, total, jobIds}>`.
   *  Resets all queue entries; clears prior runs. */
  start: (
    perQueue: Map<string, { action: BulkActionKind; total: number; jobIds: string[] }>,
  ) => void
  /** Advance a single queue's progress. Called by the global
   *  `bulk:progress` listener with the queue-disambiguated payload. */
  tick: (queueId: string, completed: number, failedSoFar: number) => void
  /** Clear all maps. Called by the BulkActionBar when the bulk
   *  resolves OR the user cancels OR the orchestrator settles. */
  finish: () => void

  /** Aggregate completed across all queues. */
  aggregateCompleted: () => number
  /** Aggregate total across all queues. */
  aggregateTotal: () => number
  /** Aggregate failed-so-far across all queues. */
  aggregateFailedSoFar: () => number

  /** Read the per-row state for a job id; returns null if the job
   *  is not part of any current bulk run. */
  rowState: (jobId: string) => PerRowState | null
}

function sum(map: Map<string, number>): number {
  let total = 0
  for (const v of map.values()) total += v
  return total
}

export const useBulkProgressStore = create<BulkProgressStore>((set, get) => ({
  runningActions: new Map(),
  perQueueCompleted: new Map(),
  perQueueFailedSoFar: new Map(),
  perRow: new Map(),

  start: (perQueue) => {
    const runningActions = new Map<string, RunningAction>()
    const perQueueCompleted = new Map<string, number>()
    const perQueueFailedSoFar = new Map<string, number>()
    const perRow = new Map<string, PerRowState>()
    for (const [queueId, { action, total, jobIds }] of perQueue.entries()) {
      runningActions.set(queueId, { action, total })
      perQueueCompleted.set(queueId, 0)
      perQueueFailedSoFar.set(queueId, 0)
      for (const id of jobIds) perRow.set(id, "queued")
    }
    set({ runningActions, perQueueCompleted, perQueueFailedSoFar, perRow })
  },

  tick: (queueId, completed, failedSoFar) => {
    set((state) => {
      const nextCompleted = new Map(state.perQueueCompleted)
      const nextFailed = new Map(state.perQueueFailedSoFar)
      nextCompleted.set(queueId, completed)
      nextFailed.set(queueId, failedSoFar)
      return { perQueueCompleted: nextCompleted, perQueueFailedSoFar: nextFailed }
    })
  },

  finish: () =>
    set({
      runningActions: new Map(),
      perQueueCompleted: new Map(),
      perQueueFailedSoFar: new Map(),
      perRow: new Map(),
    }),

  aggregateCompleted: () => sum(get().perQueueCompleted),
  aggregateTotal: () => {
    let total = 0
    for (const r of get().runningActions.values()) total += r.total
    return total
  },
  aggregateFailedSoFar: () => sum(get().perQueueFailedSoFar),

  rowState: (jobId) => get().perRow.get(jobId) ?? null,
}))
