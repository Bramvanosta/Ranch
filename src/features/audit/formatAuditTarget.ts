import type { AuditTarget } from "@/lib/tauri/bindings"

/**
 * pretty-print an `AuditTarget` for the audit log row.
 *
 * Examples:
 *   - `Job` → `"emails / #42"`
 *   - `Queue` → `"emails"`
 *   - `Scheduler` → `"emails / daily-report"`
 *   - `Connection` → `"connection"`
 *   - `BulkJobs` → `"emails / 247 jobs"`
 *
 * Compact enough for the table cell without losing the discriminator.
 */
export function formatAuditTarget(target: AuditTarget): string {
  switch (target.kind) {
    case "job":
      return `${target.data.queueName} / #${target.data.jobId}`
    case "queue":
      return target.data.queueName
    case "scheduler":
      return `${target.data.queueName} / ${target.data.schedulerKey}`
    case "connection":
      return "connection"
    case "bulkJobs":
      return `${target.data.queueName} / ${target.data.jobCount} ${target.data.jobCount === 1 ? "job" : "jobs"}`
  }
}
