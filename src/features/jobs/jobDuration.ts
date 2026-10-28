/**
 * Derive a row's "Duration" cell from BullMQ job timestamps.
 *
 * Rules:
 *   - completed / failed: `finishedOn − processedOn` (the actual work
 *     duration).
 *   - active: `now − processedOn` (live tick).
 *   - waiting / waiting children / paused / delayed / unknown:
 *     no duration applicable.
 *
 * Returns `null` when no value applies; callers render `-`.
 */
import type { Job } from "@/lib/tauri/bindings"

export function jobDurationMs(job: Job, now: number): number | null {
  if (job.processedOn === null) return null
  if (job.state === "completed" || job.state === "failed") {
    if (job.finishedOn === null) return null
    return Math.max(0, job.finishedOn - job.processedOn)
  }
  if (job.state === "active") {
    return Math.max(0, now - job.processedOn)
  }
  return null
}
