import { useQuery } from "@tanstack/react-query"

import { useRefetchInterval } from "@/features/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { commands, type QueueRef } from "@/lib/tauri/bindings"

/**
 * TanStack Query hook for paginated BullMQ `Job.log()` reads.
 *
 * The `bull:{queue}:{id}:logs` key is a Redis list; `total === 0` is the
 * legitimate "this job never logged" case (NOT an error). Default page
 * is 200 lines — covers the realistic case; pathological multi-thousand
 * cases get Previous/Next paging in the drill-down's Logs tab.
 */
export const LOGS_PAGE_SIZE = 200

export interface UseJobLogsArgs {
  connectionId: string
  queueRef: QueueRef
  jobId: string
  /** 0-based offset into the BullMQ log list. Default 0. */
  offset?: number
  /** Page size (defaults to LOGS_PAGE_SIZE). */
  limit?: number
  /** The job is running: read new lines as they come. */
  live?: boolean
}

export function useJobLogs(args: UseJobLogsArgs) {
  const offset = args.offset ?? 0
  const limit = args.limit ?? LOGS_PAGE_SIZE
  const queryKey = [
    "jobLogs",
    args.connectionId,
    args.queueRef.prefix,
    args.queueRef.name,
    args.jobId,
    offset,
    limit,
  ]
  // A running job's logs follow the refresh setting (2 s
  // floor), through one self-paced poller.
  const interval = useRefetchInterval(2_000)
  useSharedPoll(queryKey, args.live ? interval : false)
  return useQuery({
    queryKey,
    queryFn: async () => {
      const result = await commands.getJobLogs({
        connectionId: args.connectionId,
        queueRef: args.queueRef,
        jobId: args.jobId,
        offset,
        limit,
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
      if (kind === "schemaNotRecognized") return false
      return failureCount < 2
    },
  })
}
