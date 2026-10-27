import { useQuery } from "@tanstack/react-query"

import { useRefetchInterval } from "@/features/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { commands, type QueueRef } from "@/lib/tauri/bindings"

/**
 * TanStack Query hook for the single-job drill-down view.
 *
 * Backed by `get_job` (one HGETALL + decode). Cached for 30s — drill-down
 * content rarely changes within a diagnosis session; a manual refresh
 * button can call `query.refetch()`.
 *
 * `jobNotFound` and `schemaNotRecognized` are user-actionable errors that
 * surface immediately without retry. Transient failures (Redis dropped,
 * etc.) retry once.
 *
 * Watched while open, through the refresh setting: a running
 * job every 2 s (its progress, its lock), any other every
 * 10 s, so a waiting job that starts, or a failed one retried elsewhere,
 * shows it.
 */
export interface UseJobArgs {
  connectionId: string
  queueRef: QueueRef
  jobId: string
}

/** A running job's progress and lock: as often as this. */
export const ACTIVE_JOB_REFRESH_MS = 2_000
/** Any other open job: as often as this. */
export const JOB_REFRESH_MS = 10_000

export function useJob(args: UseJobArgs) {
  const queryKey = ["job", args.connectionId, args.queueRef.prefix, args.queueRef.name, args.jobId]
  const interval = useRefetchInterval(ACTIVE_JOB_REFRESH_MS)
  const query = useQuery({
    queryKey,
    queryFn: async () => {
      const result = await commands.getJob({
        connectionId: args.connectionId,
        queueRef: args.queueRef,
        jobId: args.jobId,
      })
      if (result.status === "error") {
        const err = new Error(result.error.message)
        ;(err as Error & { kind?: string }).kind = result.error.kind
        throw err
      }
      return result.data
    },
    staleTime: 30_000,
    refetchInterval: false,
    retry: (failureCount, error) => {
      const kind = (error as Error & { kind?: string }).kind
      if (kind === "jobNotFound" || kind === "schemaNotRecognized") return false
      return failureCount < 1
    },
  })
  const gone = (query.error as (Error & { kind?: string }) | null)?.kind === "jobNotFound"
  useSharedPoll(
    queryKey,
    interval === false || gone
      ? false
      : query.data?.state === "active"
        ? interval
        : Math.max(interval, JOB_REFRESH_MS),
  )
  return query
}
