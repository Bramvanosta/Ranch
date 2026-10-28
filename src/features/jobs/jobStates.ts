import { HEADLINE_STATES, isHeadline, type JobStateInfo } from "@/domain/jobStates"
import { countOf } from "@/domain/stateCounts"
import type { JobState, StateCounts } from "@/lib/tauri/bindings"

/**
 * The state tabs of a queue's job list, from the states its backend keeps
 * (`useJobStates`): the headline lifecycle states first, in the order work
 * moves through them, then the backend's other states in its order, which
 * only appear while they hold jobs (or are the one open). Most queues
 * never use them, but a job added with a priority, or to a paused queue,
 * must be findable.
 */
export function visibleStateTabs(
  states: ReadonlyArray<JobStateInfo>,
  counts: StateCounts | undefined,
  open: JobState | undefined,
): ReadonlyArray<JobStateInfo> {
  const headline = HEADLINE_STATES.flatMap((code) => states.filter((s) => s.code === code))
  const others = states.filter(
    (s) => !isHeadline(s.code) && (s.code === open || countOf(counts, s.code) > 0),
  )
  return [...headline, ...others]
}
