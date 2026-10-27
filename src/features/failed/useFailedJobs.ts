import { keepPreviousData, useQuery } from "@tanstack/react-query"

import { useRefetchInterval } from "@/features/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { commands } from "@/lib/tauri/bindings"

export const FAILED_PAGE_SIZE = 50

export interface FailedJobsFilter {
  queue: string
  name: string
  reason: string
}

export const EMPTY_FAILED_FILTER: FailedJobsFilter = { queue: "", name: "", reason: "" }

/**
 * Everything that failed on a connection, across its queues,
 * newest first. One backend call merges the queues; the page, the
 * per-queue counts and the "by job type" summary come back together.
 *
 * Refetches every 10 s: a failure is what this view exists to show, but each
 * call scans up to 500 jobs per queue, so it polls slower than the counters.
 */
export function useFailedJobs(connectionId: string, filter: FailedJobsFilter, page: number) {
  // A cross-queue scan: never faster than 10 s, whatever the setting.
  const refetchInterval = useRefetchInterval(10_000)
  // One self-paced poller per query, shared (see `useSharedPoll`).
  const queryKey = ["failed-jobs", connectionId, filter.queue, filter.name, filter.reason, page]
  useSharedPoll(queryKey, refetchInterval)
  return useQuery({
    queryKey,
    queryFn: async () => {
      const result = await commands.listFailedJobs({
        connectionId,
        offset: page * FAILED_PAGE_SIZE,
        limit: FAILED_PAGE_SIZE,
        queue: filter.queue || null,
        name: filter.name || null,
        reason: filter.reason || null,
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
