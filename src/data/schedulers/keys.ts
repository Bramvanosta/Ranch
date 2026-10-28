import type { CronJob, QueueId, Schedule } from "@/lib/tauri/bindings"

/** Query keys of job schedulers. */
export const schedulerKeys = {
  /** Every queue's schedulers on a connection. */
  all: (connectionId: string) => ["cronJobs", connectionId] as const,
  /** One queue's schedulers. */
  list: (connectionId: string, queueId: QueueId) =>
    [...schedulerKeys.all(connectionId), queueId] as const,
  /** A scheduler's runs over the next 24 hours, for the connection's lanes. */
  runs: (
    connectionId: string,
    queueId: QueueId,
    s: Pick<CronJob, "key" | "pattern" | "every" | "tz" | "nextRun">,
  ) =>
    ["schedulerRuns", connectionId, queueId, s.key, s.pattern, s.every, s.tz, s.nextRun] as const,
  /** A scheduler's next five runs, for its inspector. */
  nextRuns: (connectionId: string, queueId: QueueId, s: Pick<CronJob, "key" | "nextRun">) =>
    ["schedulerNextFive", connectionId, queueId, s.key, s.nextRun] as const,
  /** The next runs of a schedule being edited. */
  preview: (schedule: Schedule | null) => ["schedulePreview", schedule] as const,
}
