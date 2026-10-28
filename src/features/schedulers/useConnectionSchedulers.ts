import { useQueries } from "@tanstack/react-query"
import { useMemo } from "react"
import { useSchedulerPauses } from "@/data/audit/queries"
import { useQueues } from "@/data/queues/queries"
import { schedulerKeys } from "@/data/schedulers/keys"
import { fetchCronJobs, fetchRunsWithin } from "@/data/schedulers/queries"
import { useRefetchInterval } from "@/data/settings/useRefreshSettings"
import { useSharedPolls } from "@/lib/polling/sharedPoll"
import type { AuditLogEntry, CronJob, QueueId } from "@/lib/tauri/bindings"

export const DAY_MS = 24 * 60 * 60 * 1000

export interface SchedulerRow {
  queueId: QueueId
  queueName: string
  scheduler: CronJob
  /** Runs in the next 24 hours (none for a paused scheduler). */
  runs: number[]
}

/** `queue/key`, as the audit log names a scheduler (`useSchedulerPausers`). */
export const rowId = (r: Pick<SchedulerRow, "queueName" | "scheduler">) =>
  `${r.queueName}/${r.scheduler.key}`

/**
 * Every scheduler of a connection, with its runs over
 * the next 24 hours (`schedule_runs_within`, the backend's own cron reading).
 */
export function useConnectionSchedulers(connectionId: string) {
  const queues = useQueues(connectionId).data
  const refetchInterval = useRefetchInterval(10_000)
  // One self-paced poller per query (see `useSharedPoll`).
  useSharedPolls(
    (queues ?? []).map((q) => schedulerKeys.list(connectionId, q.id)),
    refetchInterval,
  )
  const lists = useQueries({
    queries: (queues ?? []).map((q) => ({
      queryKey: schedulerKeys.list(connectionId, q.id),
      // The same cached list as the queue's Schedulers tab (`useCronJobs`):
      // one key, one shape.
      queryFn: () => fetchCronJobs(connectionId, q.id),
      refetchInterval: false as const,
    })),
  })
  const pairs = useMemo(
    () =>
      lists.flatMap((l, i) => {
        const queue = queues?.[i]
        return queue && l.data
          ? l.data.map((scheduler) => ({ queueId: queue.id, queueName: queue.name, scheduler }))
          : []
      }),
    [lists, queues],
  )
  const windows = useQueries({
    queries: pairs.map(({ queueId, scheduler }) => ({
      queryKey: schedulerKeys.runs(connectionId, queueId, scheduler),
      queryFn: () => fetchRunsWithin(scheduler, DAY_MS, 500),
      enabled: !scheduler.paused,
      staleTime: 60_000,
    })),
  })
  const rows = useMemo<SchedulerRow[]>(
    () =>
      pairs
        .map((p, i) => ({ ...p, runs: p.scheduler.paused ? [] : (windows[i]?.data ?? []) }))
        .sort(
          (a, b) =>
            Number(a.scheduler.paused) - Number(b.scheduler.paused) ||
            a.scheduler.nextRun - b.scheduler.nextRun ||
            a.scheduler.key.localeCompare(b.scheduler.key),
        ),
    [pairs, windows],
  )
  const loading = !queues || lists.some((l) => l.isLoading)
  const error = lists.find((l) => l.error)?.error ?? null
  return { rows, loading, error }
}

/** Who paused each scheduler, from the audit log: `queue/key` → its entry. */
export function useSchedulerPausers(connectionId: string): Map<string, AuditLogEntry> {
  const { data } = useSchedulerPauses(connectionId)
  return useMemo(() => {
    const out = new Map<string, AuditLogEntry>()
    // Newest first: the first entry per scheduler is the pause that holds.
    for (const e of data ?? []) {
      if (e.target.kind !== "scheduler" || e.outcome !== "success") continue
      const id = `${e.target.data.queueName}/${e.target.data.schedulerKey}`
      if (!out.has(id)) out.set(id, e)
    }
    return out
  }, [data])
}
