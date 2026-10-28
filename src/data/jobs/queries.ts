import { keepPreviousData, useQueries, useQuery, useQueryClient } from "@tanstack/react-query"
import { useMemo } from "react"
import { queueKeys } from "@/data/queues/keys"
import { useQueues } from "@/data/queues/queries"
import { type CommandResult, errorKind, unwrap } from "@/data/result"
import { useRefetchInterval } from "@/data/settings/useRefreshSettings"
import { DEFAULT_PAGE_SIZE, type PageSize } from "@/domain/pageSize"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import {
  commands,
  type GetJobArgs,
  type GetJobLogsArgs,
  type Job,
  type JobLogsOutput,
  type JobSort,
  type JobState,
  type Queue,
  type QueueId,
} from "@/lib/tauri/bindings"
import { LIVE_SAFETY_NET_MS, useIsLive } from "@/stores/useLiveStore"
import { jobKeys } from "./keys"

/**
 * TanStack Query hook for the single-job drill-down view.
 *
 * Backed by `get_job` (one HGETALL + decode). Cached for 30s: drill-down
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
  queueId: QueueId
  jobId: string
}

/** A running job's progress and lock: as often as this. */
export const ACTIVE_JOB_REFRESH_MS = 2_000
/** Any other open job: as often as this. */
export const JOB_REFRESH_MS = 10_000

export function useJob(args: UseJobArgs) {
  const queryKey = jobKeys.detail(args.connectionId, args.queueId, args.jobId)
  const interval = useRefetchInterval(ACTIVE_JOB_REFRESH_MS)
  const query = useQuery({
    queryKey,
    queryFn: async () => {
      return unwrap(
        await commands.getJob({
          connectionId: args.connectionId,
          queueId: args.queueId,
          jobId: args.jobId,
        }),
      )
    },
    staleTime: 30_000,
    refetchInterval: false,
    retry: (failureCount, error) => {
      const kind = errorKind(error)
      if (kind === "jobNotFound" || kind === "schemaNotRecognized") return false
      return failureCount < 1
    },
  })
  const gone = errorKind(query.error) === "jobNotFound"
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
 *  - `staleTime: 30s`: don't auto-refetch as the user navigates between
 *    pages. The queue-counter polling at 1s is the live signal;
 *    job content rarely changes within a single drill-down session.
 *  - Placeholder: when paginating, changing state or sort, keep the
 *    previous rows visible until the new data lands, but only
 *    within the same queue: another queue's rows never show under this
 *    queue's name (they did on a slow link, with no signal).
 *  - The visible page refreshes on its own (the user's
 *    interval, at least every 5 s; not while searching: a search scans
 *    the whole state), and when its state's counter moves (`JobsPanel`).
 *  - `schemaNotRecognized` is non-retryable: surface immediately so the
 *    user sees the typed error message without waiting for retries.
 *  - `superseded` is non-retryable too: the actor stopped that
 *    scan because a newer one was requested. Retrying it would in turn
 *    supersede the query the user is actually waiting for.
 */
export interface UseJobsArgs {
  connectionId: string
  queueId: QueueId
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

/** `["jobs", connectionId, queueId, …]`: the queue a list key is for. */
const queueOfKey = (key: readonly unknown[]) => `${key[1]}\u0000${key[2]}`

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
    ...jobKeys.listsInState(args.connectionId, args.queueId, args.state),
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
      return unwrap(
        await commands.listJobs({
          connectionId: args.connectionId,
          queueId: args.queueId,
          state: args.state,
          offset: args.page * pageSize,
          limit: pageSize,
          newestFirst: args.newestFirst ?? true,
          search: search === "" ? null : search,
          reason: reason === "" ? null : reason,
          sort,
        }),
      )
    },
    enabled: args.enabled ?? true,
    staleTime: 30_000,
    refetchInterval: false,
    placeholderData: (previous, previousQuery) =>
      previousQuery &&
      queueOfKey(previousQuery.queryKey) ===
        queueOfKey(jobKeys.lists(args.connectionId, args.queueId))
        ? previous
        : undefined,
    retry: (failureCount, error) => {
      const kind = errorKind(error)
      if (kind === "schemaNotRecognized" || kind === "superseded") return false
      return failureCount < 2
    },
  })
}

/**
 * TanStack Query hook for paginated BullMQ `Job.log()` reads.
 *
 * The `bull:{queue}:{id}:logs` key is a Redis list; `total === 0` is the
 * legitimate "this job never logged" case (NOT an error). Default page
 * is 200 lines, which covers the realistic case; pathological multi-thousand
 * cases get Previous/Next paging in the drill-down's Logs tab.
 */
export const LOGS_PAGE_SIZE = 200

export interface UseJobLogsArgs {
  connectionId: string
  queueId: QueueId
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
  const queryKey = jobKeys.logs(args.connectionId, args.queueId, args.jobId, offset, limit)
  // A running job's logs follow the refresh setting (2 s
  // floor), through one self-paced poller.
  const interval = useRefetchInterval(2_000)
  useSharedPoll(queryKey, args.live ? interval : false)
  return useQuery({
    queryKey,
    queryFn: async () => {
      return unwrap(
        await commands.getJobLogs({
          connectionId: args.connectionId,
          queueId: args.queueId,
          jobId: args.jobId,
          offset,
          limit,
        }),
      )
    },
    staleTime: 30_000,
    refetchInterval: false,
    retry: (failureCount, error) => {
      const kind = errorKind(error)
      if (kind === "schemaNotRecognized") return false
      return failureCount < 2
    },
  })
}

/**
 * Orchestrates parallel `list_jobs` fetches across every
 * queue in a connection so the user can filter jobs across the whole
 * connection.
 *
 * Per-queue page size is fixed at 50 (same as `useJobs`). Pagination
 * within cross-queue scope is intentionally off: refining with the filter
 * is the intended path.
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
    .map((q) => q.id)
    .slice()
    .sort()

  return useQuery({
    queryKey: jobKeys.crossQueue(connectionId, state, queueNames),
    queryFn: async (): Promise<CrossQueueJobsResult> => {
      if (!queues || queues.length === 0) {
        return { jobs: [], aggregateTotal: 0, perQueueErrors: [] }
      }
      const settled = await Promise.allSettled(
        queues.map((q) =>
          commands.listJobs({
            connectionId,
            queueId: q.id,
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
        const qName = queues[i].name
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
      // breaks that, so re-sort globally by `timestamp` descending so the
      // cross-queue list still reads newest-first across all sources.
      jobs.sort((a, b) => b.timestamp - a.timestamp)
      return { jobs, aggregateTotal, perQueueErrors }
    },
    enabled: enabled && queues !== undefined,
    staleTime: 30_000,
  })
}

/** One job, as a plain call: a search that probes several queues for an id. */
export function getJob(args: GetJobArgs): Promise<CommandResult<Job>> {
  return commands.getJob(args)
}

/** A page of a job's log lines, as a plain call. */
export function getJobLogs(args: GetJobLogsArgs): Promise<CommandResult<JobLogsOutput>> {
  return commands.getJobLogs(args)
}

export const RECENT_JOBS_LIMIT = 10

/**
 * What ran last, on one queue, or across the queues of a connection when
 * `queueId` is null. One backend call (`list_recent_jobs`): the running jobs,
 * then the latest completed / failed ones.
 *
 * Never polled faster than every 5 s: across a connection it reads three keys
 * per queue, and "the last jobs" does not need the counters' cadence.
 */
export function useRecentJobs(connectionId: string, queueId: QueueId | null) {
  const refetchInterval = useRefetchInterval(5_000)
  // One self-paced poller per query, shared (see `useSharedPoll`).
  const queryKey = jobKeys.recentList(connectionId, queueId)
  useSharedPoll(queryKey, refetchInterval)
  return useQuery({
    queryKey,
    queryFn: async () => {
      return unwrap(
        await commands.listRecentJobs({
          connectionId,
          queueId,
          limit: RECENT_JOBS_LIMIT,
        }),
      )
    },
    placeholderData: keepPreviousData,
    refetchInterval: false,
    retry: (failureCount, error) => errorKind(error) !== "schemaNotRecognized" && failureCount < 2,
  })
}

/**
 * The latest 50 jobs of each of `queueIds`, by queue id: what a scheduler
 * ran last, or what a worker processed.
 */
export function useLatestJobsOf(
  connectionId: string,
  queueIds: readonly QueueId[],
): Map<QueueId, Job[]> {
  const results = useQueries({
    queries: queueIds.map((queueId) => ({
      queryKey: jobKeys.latest(connectionId, queueId),
      queryFn: async () => {
        const r = unwrap(await commands.listRecentJobs({ connectionId, queueId, limit: 50 }))
        return r.jobs
      },
      staleTime: 10_000,
    })),
  })
  return useMemo(() => {
    const out = new Map<QueueId, Job[]>()
    queueIds.forEach((q, i) => {
      out.set(q, results[i]?.data ?? [])
    })
    return out
  }, [queueIds, results])
}

/**
 * A job id looked up in the queues Ranch has listed: those of
 * `connectionId`, or of every listed connection when it is null (at most 40
 * queues).
 */
export function useJobIdSearch(
  connectionId: string | null,
  jobId: string | null,
  enabled: boolean,
) {
  const queryClient = useQueryClient()
  return useQuery({
    queryKey: jobKeys.search(connectionId, jobId),
    queryFn: async () => {
      const targets = queryClient
        .getQueriesData<Queue[]>({ queryKey: queueKeys.all })
        .flatMap(([key, list]) => {
          const listed = String(key[1] ?? "")
          if (connectionId && listed !== connectionId) return []
          return Array.isArray(list)
            ? list.map((q) => ({ connectionId: listed, queueId: q.id }))
            : []
        })
        .slice(0, 40)
      const found = await Promise.all(
        targets.map(async (t) => {
          const r = await commands.getJob({ ...t, jobId: jobId ?? "" })
          return r.status === "ok" ? { ...t, job: r.data } : null
        }),
      )
      return found.filter((f) => f !== null)
    },
    enabled: enabled && jobId !== null,
    staleTime: 5_000,
  })
}
