import { useQuery } from "@tanstack/react-query"

import { useQueues } from "@/features/queues/useQueues"
import { commands, type Job, type JobState } from "@/lib/tauri/bindings"

/**
 * Orchestrates parallel `list_jobs` fetches across every
 * queue in a connection so the user can filter jobs across the whole
 * connection.
 *
 * Per-queue page size is fixed at 50 (same as `useJobs`). Pagination
 * within cross-queue scope is intentionally disabled in v1 — refining
 * with the filter is the intended path; full cross-queue paging can
 * land in a v1.x story if usage demands it.
 *
 * Per-queue failures are tolerated: a single rejected query contributes
 * an entry to `perQueueErrors` and 0 jobs. The hook never throws.
 */
export const CROSS_QUEUE_PER_QUEUE_LIMIT = 50

export interface UseCrossQueueJobsArgs {
  connectionId: string
  state: JobState
  /** `false` while the route is in single-queue scope: no fan-out fetch. */
  enabled?: boolean
}

export interface CrossQueueJobsResult {
  jobs: Job[]
  aggregateTotal: number
  perQueueErrors: Array<{ queueName: string; message: string }>
}

export function useCrossQueueJobs({ connectionId, state, enabled = true }: UseCrossQueueJobsArgs) {
  const { data: queues } = useQueues(connectionId)
  // Sorted queue-name list as part of the queryKey so TanStack Query
  // re-runs the fetch when the set of queues changes (e.g. a new queue
  // appears via the 1s `useQueues` poll).
  const queueNames = (queues ?? [])
    .map((q) => q.queueRef.name)
    .slice()
    .sort()

  return useQuery({
    queryKey: ["crossQueueJobs", connectionId, state, queueNames.join(",")],
    queryFn: async (): Promise<CrossQueueJobsResult> => {
      if (!queues || queues.length === 0) {
        return { jobs: [], aggregateTotal: 0, perQueueErrors: [] }
      }
      const settled = await Promise.allSettled(
        queues.map((q) =>
          commands.listJobs({
            connectionId,
            queueRef: q.queueRef,
            state,
            offset: 0,
            limit: CROSS_QUEUE_PER_QUEUE_LIMIT,
            newestFirst: true,
          }),
        ),
      )

      const jobs: Job[] = []
      const perQueueErrors: Array<{ queueName: string; message: string }> = []
      let aggregateTotal = 0
      for (const [i, r] of settled.entries()) {
        const qName = queues[i].queueRef.name
        if (r.status === "rejected") {
          console.warn(`cross-queue listJobs rejected for ${qName}:`, r.reason)
          perQueueErrors.push({ queueName: qName, message: String(r.reason) })
          continue
        }
        if (r.value.status === "error") {
          console.warn(`cross-queue listJobs error for ${qName}:`, r.value.error)
          perQueueErrors.push({ queueName: qName, message: r.value.error.message })
          continue
        }
        jobs.push(...r.value.data.jobs)
        aggregateTotal += Number(r.value.data.total)
      }
      // BullMQ returns per-queue newest-first ordering, but the merge
      // breaks that — re-sort globally by `timestamp` descending so the
      // cross-queue list still reads newest-first across all sources.
      jobs.sort((a, b) => b.timestamp - a.timestamp)
      return { jobs, aggregateTotal, perQueueErrors }
    },
    enabled: enabled && queues !== undefined,
    staleTime: 30_000,
  })
}
