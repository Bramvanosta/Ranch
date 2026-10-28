import { keepPreviousData, useQuery } from "@tanstack/react-query"
import { errorKind, unwrap } from "@/data/result"
import { useRefetchInterval } from "@/data/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { commands } from "@/lib/tauri/bindings"
import { jobKeys } from "./keys"

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
  const queryKey = jobKeys.failedPage(connectionId, filter, page)
  useSharedPoll(queryKey, refetchInterval)
  return useQuery({
    queryKey,
    queryFn: async () => {
      return unwrap(
        await commands.listFailedJobs({
          connectionId,
          offset: page * FAILED_PAGE_SIZE,
          limit: FAILED_PAGE_SIZE,
          queue: filter.queue || null,
          name: filter.name || null,
          reason: filter.reason || null,
        }),
      )
    },
    placeholderData: keepPreviousData,
    refetchInterval: false,
    retry: (failureCount, error) => errorKind(error) !== "schemaNotRecognized" && failureCount < 2,
  })
}

/** `list_failure_causes` of one queue (by name), or of the whole connection (`null`). */
export async function fetchFailureCauses(connectionId: string, queueName: string | null) {
  return unwrap(await commands.listFailureCauses({ connectionId, queue: queueName }))
}

export interface UseFailureCausesOptions {
  /** Polled through the refresh setting, never faster than this; `false`: read once. */
  floorMs?: number | false
  enabled?: boolean
}

/**
 * What failed, grouped by cause (the failure reason with its values
 * normalised), most frequent first: one queue's (by name), or the whole
 * connection's (`null`).
 */
export function useFailureCauses(
  connectionId: string,
  queueName: string | null,
  { floorMs = false, enabled = true }: UseFailureCausesOptions = {},
) {
  const queryKey = jobKeys.failureCauseList(connectionId, queueName ?? "")
  const interval = useRefetchInterval(floorMs === false ? 0 : floorMs)
  // One self-paced poller per query, shared (see `useSharedPoll`).
  useSharedPoll(queryKey, floorMs === false || !enabled ? false : interval)
  return useQuery({
    queryKey,
    queryFn: () => fetchFailureCauses(connectionId, queueName),
    enabled,
    refetchInterval: false,
  })
}

/** The failed jobs of one cause in a queue, the first `limit` of them. */
export function useCauseJobs(
  connectionId: string,
  queueName: string,
  signature: string,
  limit: number,
) {
  return useQuery({
    queryKey: jobKeys.causeJobs(connectionId, queueName, signature),
    queryFn: async () => {
      return unwrap(
        await commands.listFailedJobs({
          connectionId,
          offset: 0,
          limit,
          queue: queueName,
          name: null,
          reason: null,
          signature,
        }),
      )
    },
  })
}
