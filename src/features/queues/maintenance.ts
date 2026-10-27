import type {
  CleanState,
  MaintenancePreview,
  MaintenanceResult,
  QueueMaintenance,
} from "@/lib/tauri/bindings"

/**
 * Queue maintenance, as data (pure, unit-tested): the three
 * operations, the "older than" choices, and the sentences the dialog shows
 * before (preview) and after (result).
 */
export type MaintenanceKind = QueueMaintenance["kind"]

export const OLDER_THAN_OPTIONS: ReadonlyArray<{ label: string; ms: number }> = [
  { label: "All of them", ms: 0 },
  { label: "Older than 1 hour", ms: 3_600_000 },
  { label: "Older than 1 day", ms: 86_400_000 },
  { label: "Older than 7 days", ms: 7 * 86_400_000 },
  { label: "Older than 30 days", ms: 30 * 86_400_000 },
]

export const CLEAN_STATES: ReadonlyArray<{ value: CleanState; label: string }> = [
  { value: "completed", label: "Completed" },
  { value: "failed", label: "Failed" },
]

export interface MaintenanceCopy {
  title: string
  /** What the operation does, and what it never touches. */
  description: string
  confirmLabel: string
  /** Data is lost: the confirm button is styled as dangerous. */
  destructive: boolean
}

export function maintenanceCopy(kind: MaintenanceKind, queueName: string): MaintenanceCopy {
  switch (kind) {
    case "drain":
      return {
        title: `Drain ${queueName}`,
        description:
          "Removes every job that has not started: waiting, paused, prioritized and delayed. Active, completed and failed jobs stay. This cannot be undone.",
        confirmLabel: "Drain",
        destructive: true,
      }
    case "clean":
      return {
        title: `Clean ${queueName}`,
        description:
          "Removes finished jobs — their payload, logs and result with them. This cannot be undone.",
        confirmLabel: "Clean",
        destructive: true,
      }
    case "retryAllFailed":
      return {
        title: `Retry all failed jobs of ${queueName}`,
        description:
          "Puts every failed job back in line, as the Retry button does for one. Workers pick them up again; a job that fails again comes back to Failed.",
        confirmLabel: "Retry all",
        destructive: false,
      }
  }
}

const jobs = (n: number) => `${n.toLocaleString()} ${n === 1 ? "job" : "jobs"}`

/** The preview line: the affected count, before anything runs. */
export function describePreview(op: QueueMaintenance, preview: MaintenancePreview): string {
  if (preview.affected === 0) {
    return op.kind === "retryAllFailed"
      ? "No failed job to retry."
      : op.kind === "clean"
        ? "No job matches."
        : "Nothing to drain."
  }
  const spared =
    preview.sparedSchedulerJobs > 0
      ? ` The next run of ${preview.sparedSchedulerJobs === 1 ? "a scheduler" : `${preview.sparedSchedulerJobs} schedulers`} is left in place.`
      : ""
  switch (op.kind) {
    case "drain":
      return `${jobs(preview.affected)} will be removed.${spared}`
    case "clean":
      return `${jobs(preview.affected)} will be removed.`
    case "retryAllFailed":
      return `${jobs(preview.affected)} will be retried.`
  }
}

/** The result line, for the toast. */
export function describeResult(result: MaintenanceResult): string {
  const verb = result.action === "retry" ? "retried" : "removed"
  const parts = [`${jobs(result.affected)} ${verb}`]
  if (result.skipped > 0) {
    parts.push(
      result.action === "retry"
        ? `${result.skipped.toLocaleString()} no longer existed`
        : `${result.skipped.toLocaleString()} left in place (flows, scheduler runs)`,
    )
  }
  if (result.cancelled) parts.push("interrupted before the end")
  return parts.join(" · ")
}
