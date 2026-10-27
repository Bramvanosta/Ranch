import { keepPreviousData, useQuery } from "@tanstack/react-query"

import { useRefetchInterval } from "@/features/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { commands, type QueueRef } from "@/lib/tauri/bindings"

export const RECENT_JOBS_LIMIT = 10

/**
 * What ran last — on one queue, or across the queues of a connection when
 * `queueRef` is null. One backend call (`list_recent_jobs`): the running jobs,
 * then the latest completed / failed ones.
 *
 * Never polled faster than every 5 s: across a connection it reads three keys
 * per queue, and "the last jobs" does not need the counters' cadence.
 */
export function useRecentJobs(connectionId: string, queueRef: QueueRef | null) {
  const refetchInterval = useRefetchInterval(5_000)
  // One self-paced poller per query, shared (see `useSharedPoll`).
  const queryKey = ["recent-jobs", connectionId, queueRef?.prefix ?? null, queueRef?.name ?? null]
  useSharedPoll(queryKey, refetchInterval)
  return useQuery({
    queryKey,
    queryFn: async () => {
      const result = await commands.listRecentJobs({
        connectionId,
        queueRef,
        limit: RECENT_JOBS_LIMIT,
      })
      if (result.status === "error") {
        const err = new Error(result.error.message)
        ;(err as Error & { kind?: string }).kind = result.error.kind
        throw err
      }
      return result.data
    },
    placeholderData: keepPreviousData,
    refetchInterval: false,
    retry: (failureCount, error) =>
      (error as Error & { kind?: string }).kind !== "schemaNotRecognized" && failureCount < 2,
  })
}
