import type { JobState, JobStateInfo, StateCounts } from "@/lib/tauri/bindings"
import { waitingStates } from "./jobStates"

/**
 * The jobs a queue holds in `state`. Counts come keyed by state code and a
 * backend leaves out a state it did not count, so every reader goes
 * through here and reads an absent state as zero.
 */
export function countOf(counts: StateCounts | null | undefined, state: JobState): number {
  return Number(counts?.[state] ?? 0)
}

/**
 * The jobs in line for a worker: in the states `states` describes as
 * waiting for one (`waitingStates`), and, when asked, held by a pause.
 */
export function inLine(
  counts: StateCounts | null | undefined,
  states: ReadonlyArray<JobStateInfo>,
  withPaused = false,
): number {
  const waiting = waitingStates(states).reduce((n, state) => n + countOf(counts, state), 0)
  return waiting + (withPaused ? countOf(counts, "paused") : 0)
}

/** The jobs in `state`, or `undefined` when the counts do not say (not read yet, or not counted). */
export function knownCount(
  counts: StateCounts | null | undefined,
  state: JobState,
): number | undefined {
  return counts && state in counts ? countOf(counts, state) : undefined
}
