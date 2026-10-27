import { useQuery } from "@tanstack/react-query"
import { LIVE_SAFETY_NET_MS, useIsLive } from "@/features/live/useConnectionEvents"
import { useRefetchInterval } from "@/features/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { commands, type JobSort, type JobState, type QueueRef } from "@/lib/tauri/bindings"

/**
 * TanStack Query hook for the paginated job list.
 *
 * Hits `list_jobs`, the first Tauri command to call `JobDecoder`.
 * Default page size is 50 (budget: `<100ms` per page render).
 *
 * `search` (job id or name), `reason` (failure reason) and
 * `sort` run **server-side across the whole state** (bounded scan), so a
 * job on page 3 is found from page 1. `pageSize` is one of `PAGE_SIZES`.
 *
 * Caching nuances:
 *  - `staleTime: 30s` — don't auto-refetch as the user navigates between
 *    pages. The queue-counter polling at 1s is the live signal;
 *    job content rarely changes within a single drill-down session.
 *  - Placeholder: when paginating, changing state or sort, keep the
 *    previous rows visible until the new data lands, but only
 *    within the same queue: another queue's rows never show under this
 *    queue's name (they did on a slow link, with no signal).
 *  - The visible page refreshes on its own (the user's
 *    interval, at least every 5 s; not while searching: a search scans
 *    the whole state), and when its state's counter moves (`JobsPanel`).
 *  - `schemaNotRecognized` is non-retryable — surface immediately so the
 *    user sees the typed error message without waiting for retries.
 *  - `superseded` is non-retryable too: the actor stopped that
 *    scan because a newer one was requested. Retrying it would in turn
 *    supersede the query the user is actually waiting for.
 */
export const PAGE_SIZES = [25, 50, 100, 250] as const
export type PageSize = (typeof PAGE_SIZES)[number]
export const DEFAULT_PAGE_SIZE: PageSize = 50

export function isPageSize(value: unknown): value is PageSize {
  return PAGE_SIZES.includes(value as PageSize)
}

export interface UseJobsArgs {
  connectionId: string
  queueRef: QueueRef
  state: JobState
  page: number
  newestFirst?: boolean
  /** server-side: substring of the job id or name. */
  search?: string
  /** server-side: substring of the failure reason. */
  reason?: string
  /** server-side sort; `null` = the state's natural order. */
  sort?: JobSort | null
  pageSize?: PageSize
  /** `false` skips the fetch (the caller owns the data). */
  enabled?: boolean
}

/** The job list's own refresh: a page read is a few round trips. */
export const JOBS_REFRESH_FLOOR_MS = 5_000

/** `["jobs", connectionId, prefix, queue, …]`: the queue a list key is for. */
const queueOfKey = (key: readonly unknown[]) => `${key[1]}\u0000${key[2]}\u0000${key[3]}`

export function useJobs(args: UseJobsArgs) {
  const pageSize = args.pageSize ?? DEFAULT_PAGE_SIZE
  const search = args.search?.trim() ?? ""
  const reason = args.reason?.trim() ?? ""
  const sort = args.sort ?? null
  const setting = useRefetchInterval(JOBS_REFRESH_FLOOR_MS)
  // Followed live, the list is read when its queue moves.
  const live = useIsLive(args.connectionId)
  const interval = setting === false || !live ? setting : Math.max(setting, LIVE_SAFETY_NET_MS)
  const scanning = search !== "" || reason !== "" || sort !== null
  const queryKey = [
    "jobs",
    args.connectionId,
    args.queueRef.prefix,
    args.queueRef.name,
    args.state,
    args.page,
    args.newestFirst ?? true,
    pageSize,
    search,
    reason,
    sort?.field ?? null,
    sort?.descending ?? false,
  ]
  // One self-paced poller per query (see `useSharedPoll`).
  useSharedPoll(queryKey, scanning || args.enabled === false ? false : interval)
  return useQuery({
    queryKey,
    queryFn: async () => {
      const result = await commands.listJobs({
        connectionId: args.connectionId,
        queueRef: args.queueRef,
        state: args.state,
        offset: args.page * pageSize,
        limit: pageSize,
        newestFirst: args.newestFirst ?? true,
        search: search === "" ? null : search,
        reason: reason === "" ? null : reason,
        sort,
      })
      if (result.status === "error") {
        const err = new Error(result.error.message)
        ;(err as Error & { kind?: string }).kind = result.error.kind
        throw err
      }
      return result.data
    },
    enabled: args.enabled ?? true,
    staleTime: 30_000,
    refetchInterval: false,
    placeholderData: (previous, previousQuery) =>
      previousQuery &&
      queueOfKey(previousQuery.queryKey) ===
        queueOfKey(["jobs", args.connectionId, args.queueRef.prefix, args.queueRef.name])
        ? previous
        : undefined,
    retry: (failureCount, error) => {
      const kind = (error as Error & { kind?: string }).kind
      if (kind === "schemaNotRecognized" || kind === "superseded") return false
      return failureCount < 2
    },
  })
}
