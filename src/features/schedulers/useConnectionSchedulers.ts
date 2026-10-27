import { useQueries, useQuery } from "@tanstack/react-query"
import { useMemo } from "react"

import { useQueues } from "@/features/queues/useQueues"
import { useRefetchInterval } from "@/features/settings/useRefreshSettings"
import { useSharedPolls } from "@/lib/polling/sharedPoll"
import {
  type AuditLogEntry,
  type CronJob,
  commands,
  type Job,
  type QueueRef,
} from "@/lib/tauri/bindings"

export const DAY_MS = 24 * 60 * 60 * 1000

export interface SchedulerRow {
  queueRef: QueueRef
  scheduler: CronJob
  /** Runs in the next 24 hours (none for a paused scheduler). */
  runs: number[]
}

export const rowId = (r: Pick<SchedulerRow, "queueRef" | "scheduler">) =>
  `${r.queueRef.name}/${r.scheduler.key}`

/**
 * Every scheduler of a connection, with its runs over
 * the next 24 hours (`schedule_runs_within`, the backend's own cron reading).
 */
export function useConnectionSchedulers(connectionId: string) {
  const queues = useQueues(connectionId).data
  const refetchInterval = useRefetchInterval(10_000)
  // One self-paced poller per query (see `useSharedPoll`).
  useSharedPolls(
    (queues ?? []).map((q) => ["cronJobs", connectionId, q.queueRef.prefix, q.queueRef.name]),
    refetchInterval,
  )
  const lists = useQueries({
    queries: (queues ?? []).map((q) => ({
      queryKey: ["cronJobs", connectionId, q.queueRef.prefix, q.queueRef.name],
      queryFn: async () => {
        const r = await commands.listCronJobs({ connectionId, queueRef: q.queueRef })
        if (r.status === "error") throw new Error(r.error.message)
        return { queueRef: q.queueRef, schedulers: r.data }
      },
      refetchInterval: false as const,
    })),
  })
  const pairs = useMemo(
    () =>
      lists.flatMap((l) =>
        l.data
          ? l.data.schedulers.map((scheduler) => ({ queueRef: l.data.queueRef, scheduler }))
          : [],
      ),
    [lists],
  )
  const windows = useQueries({
    queries: pairs.map(({ queueRef, scheduler }) => ({
      queryKey: [
        "schedulerRuns",
        connectionId,
        queueRef.name,
        scheduler.key,
        scheduler.pattern,
        scheduler.every,
        scheduler.tz,
        scheduler.nextRun,
      ],
      queryFn: async () => {
        const schedule =
          scheduler.every !== null
            ? { kind: "every" as const, everyMs: scheduler.every }
            : { kind: "pattern" as const, pattern: scheduler.pattern ?? "", tz: scheduler.tz }
        const r = await commands.scheduleRunsWithin({
          schedule,
          horizonMs: DAY_MS,
          firstMs: scheduler.nextRun,
          max: 500,
        })
        if (r.status === "error") throw new Error(r.error.message)
        return r.data.nextRunsMs
      },
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
  const { data } = useQuery({
    queryKey: ["schedulerPausers", connectionId],
    queryFn: async () => {
      const r = await commands.listAuditLogEntries({
        from: null,
        to: null,
        connectionId,
        action: "pauseScheduler",
        source: null,
        offset: 0,
        limit: 100,
      })
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    staleTime: 30_000,
  })
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

/** The latest jobs of the queues that hold schedulers: their last runs. */
export function useRecentJobsOf(connectionId: string, queues: readonly QueueRef[]) {
  const results = useQueries({
    queries: queues.map((queueRef) => ({
      queryKey: ["recentJobs", connectionId, queueRef.prefix, queueRef.name, 50],
      queryFn: async () => {
        const r = await commands.listRecentJobs({ connectionId, queueRef, limit: 50 })
        if (r.status === "error") throw new Error(r.error.message)
        return r.data.jobs
      },
      staleTime: 10_000,
    })),
  })
  return useMemo(() => {
    const out = new Map<string, Job[]>()
    queues.forEach((q, i) => {
      out.set(q.name, results[i]?.data ?? [])
    })
    return out
  }, [queues, results])
}
