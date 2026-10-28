import { isHeadline, type JobStateInfo, TONE_DOT } from "@/domain/jobStates"
import { countOf } from "@/domain/stateCounts"
import type { JobState, Queue, StateCounts } from "@/lib/tauri/bindings"

/** Jobs per state, summed over the queues (one queue = its own counts), by state code. */
export function sumCounters(queues: ReadonlyArray<Pick<Queue, "counts">>): StateCounts {
  const totals: Record<string, number> = {}
  for (const q of queues) {
    for (const [state, n] of Object.entries(q.counts ?? {})) {
      totals[state] = (totals[state] ?? 0) + Number(n ?? 0)
    }
  }
  return totals
}

export interface TotalState {
  code: JobState
  label: string
  /** The state's color (the same tokens as the chips and the charts). */
  dotClass: string
}

/**
 * The states to total, in the order the backend gives them: the headline
 * states it keeps always, the others only while they hold jobs (most
 * queues never use them).
 */
export function totalStates(
  states: ReadonlyArray<JobStateInfo>,
  totals: StateCounts | null | undefined,
): TotalState[] {
  return states
    .filter((s) => isHeadline(s.code) || countOf(totals, s.code) > 0)
    .map((s) => ({ code: s.code, label: s.label, dotClass: TONE_DOT[s.tone] }))
}
