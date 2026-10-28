import { useQuery } from "@tanstack/react-query"
import { errorKind, unwrap } from "@/data/result"
import { type CronJob, commands, type QueueId, type Schedule } from "@/lib/tauri/bindings"
import { schedulerKeys } from "./keys"

/**
 * TanStack Query hook for the CRON / repeatable jobs view.
 *
 * Hits `list_cron_jobs` which reads the `bull:{queue}:repeat` ZSET +
 * pipelines HGETALL per scheduler. Cached 30s; `schemaNotRecognized`
 * surfaces immediately without retry (same contract as `useQueues`/
 * `useJobs`/`useJob`); transient failures retry up to 2x.
 */
export interface UseCronJobsArgs {
  connectionId: string
  queueId: QueueId
  /** `false` when the connection's backend lists no schedulers. */
  enabled?: boolean
}

export function useCronJobs(args: UseCronJobsArgs) {
  return useQuery({
    queryKey: schedulerKeys.list(args.connectionId, args.queueId),
    queryFn: async () => {
      return unwrap(
        await commands.listCronJobs({
          connectionId: args.connectionId,
          queueId: args.queueId,
        }),
      )
    },
    staleTime: 30_000,
    enabled: args.enabled ?? true,
    retry: (failureCount, error) => {
      const kind = errorKind(error)
      if (kind === "schemaNotRecognized") return false
      return failureCount < 2
    },
  })
}

/**
 * The next runs of a schedule being edited, so the form can show them and
 * reject a pattern that never fires. Never served from cache.
 */
export function useSchedulePreview(schedule: Schedule | null, enabled: boolean) {
  return useQuery({
    queryKey: schedulerKeys.preview(schedule),
    queryFn: async () => {
      if (!schedule) throw new Error("incomplete")
      return unwrap(await commands.previewSchedule(schedule)).nextRunsMs
    },
    enabled: enabled && schedule !== null,
    retry: false,
    staleTime: 0,
    gcTime: 0,
  })
}

/** One queue's schedulers, for a query that reads every queue of a connection. */
export async function fetchCronJobs(connectionId: string, queueId: QueueId) {
  return unwrap(await commands.listCronJobs({ connectionId, queueId }))
}

/**
 * When a scheduler runs next, from its next run on, within `horizonMs` and
 * at most `max` times: the backend's own cron reading (`schedule_runs_within`).
 */
export async function fetchRunsWithin(
  scheduler: Pick<CronJob, "every" | "pattern" | "tz" | "nextRun">,
  horizonMs: number,
  max: number,
): Promise<number[]> {
  const schedule: Schedule =
    scheduler.every !== null
      ? { kind: "every", everyMs: scheduler.every }
      : { kind: "pattern", pattern: scheduler.pattern ?? "", tz: scheduler.tz }
  const r = unwrap(
    await commands.scheduleRunsWithin({
      schedule,
      horizonMs,
      firstMs: scheduler.nextRun,
      max,
    }),
  )
  return r.nextRunsMs
}
