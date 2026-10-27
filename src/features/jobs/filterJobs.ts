import type { Job } from "@/lib/tauri/bindings"

/**
 * client-side substring filter applied after each keystroke.
 *
 * This now serves the **cross-queue scope** only; in queue scope
 * the same two segments run server-side across the whole state (`list_jobs`
 * `search` / `reason`). The semantics are kept identical.
 *
 * Two segments:
 *   - `name`: case-insensitive substring against `job.name` or the job id.
 *   - `error`: case-insensitive substring against `job.failedReason`
 *     OR the first line of `job.stacktrace` (BullMQ stores the error
 *     name + message on line 0 — e.g. `"Error: Connection refused"`).
 *
 * Both segments active → AND semantics. Whitespace-only segments are
 * treated as empty. NO debounce: pure synchronous filter targeting
 * <16ms per keystroke (Raycast-class responsiveness).
 */
export interface JobFilterCriteria {
  /** Case-insensitive substring match against `job.name` or `job.jobRef.id`. */
  name?: string
  /** Case-insensitive substring match against `failedReason` OR `stacktrace[0]`. */
  error?: string
}

export function filterJobs(jobs: readonly Job[], criteria: JobFilterCriteria): Job[] {
  const name = criteria.name?.trim().toLowerCase() ?? ""
  const err = criteria.error?.trim().toLowerCase() ?? ""
  if (!name && !err) return [...jobs]
  return jobs.filter((j) => {
    if (name && !j.name.toLowerCase().includes(name) && !j.jobRef.id.toLowerCase().includes(name)) {
      return false
    }
    if (err) {
      const reasonHit = j.failedReason?.toLowerCase().includes(err) ?? false
      const stackHit = j.stacktrace.length > 0 && j.stacktrace[0].toLowerCase().includes(err)
      if (!reasonHit && !stackHit) return false
    }
    return true
  })
}
