import type { BulkActionKind, CrossQueueBulkResult } from "@/data/jobs/bulkMutations"
import type { BulkActionResult } from "@/lib/tauri/bindings"

/**
 * The bulk action bar as a state machine:
 *   - `selecting`: "{N} selected across {K} queues", the actions and Cancel
 *   - `preview`:   the confirmation sheet
 *   - `executing`: progress, per queue when there are several
 *   - `complete`:  the summary, dismissed after 6 s or by the next gesture
 */

/** What the summary shows: one queue's bulk result, or one per queue. */
export type CompleteResult =
  | { mode: "single"; result: BulkActionResult }
  | { mode: "cross"; result: CrossQueueBulkResult }

export type BarState =
  | { kind: "selecting" }
  | { kind: "preview"; action: BulkActionKind }
  | { kind: "executing"; action: BulkActionKind }
  | { kind: "complete"; action: BulkActionKind; result: CompleteResult }

export type BarAction =
  | { type: "preview"; action: BulkActionKind }
  | { type: "cancel" }
  | { type: "execute"; action: BulkActionKind }
  | { type: "complete"; action: BulkActionKind; result: CompleteResult }
  | { type: "reset" }

export function bulkBarReducer(state: BarState, action: BarAction): BarState {
  switch (action.type) {
    case "preview":
      if (state.kind !== "selecting") return state
      return { kind: "preview", action: action.action }
    case "cancel":
      return { kind: "selecting" }
    case "execute":
      if (state.kind !== "preview") return state
      return { kind: "executing", action: action.action }
    case "complete":
      return { kind: "complete", action: action.action, result: action.result }
    case "reset":
      return { kind: "selecting" }
  }
}

/**
 * A single-queue call that failed as a whole (a locked connection, a lost
 * link) reads as a summary with one failure carrying the reason: for
 * `connectionLocked` the message already says how to unlock.
 */
export function failedSummary(
  action: BulkActionKind,
  total: number,
  reason: string,
): CompleteResult {
  return {
    mode: "single",
    result: { action, total, succeeded: 0, failed: [{ jobId: "", reason }], cancelled: false },
  }
}

/** What had happened when the user stopped waiting (the actor carries on). */
export function cancelledSummary(
  action: BulkActionKind,
  progress: { total: number; completed: number; failedSoFar: number },
): CompleteResult {
  return {
    mode: "single",
    result: {
      action,
      total: progress.total,
      succeeded: Math.max(progress.completed - progress.failedSoFar, 0),
      failed: [],
      cancelled: true,
    },
  }
}
