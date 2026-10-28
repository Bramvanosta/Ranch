import { useQueryClient } from "@tanstack/react-query"
import { type ReactNode, useEffect, useMemo, useRef } from "react"
import { useJobStates } from "@/capabilities"
import { jobKeys } from "@/data/jobs/keys"
import { CROSS_QUEUE_PER_QUEUE_LIMIT, useCrossQueueJobs, useJobs } from "@/data/jobs/queries"
import { useQueues } from "@/data/queues/queries"
import { stateLabel } from "@/domain/jobStates"
import type { PageSize } from "@/domain/pageSize"
import { knownCount } from "@/domain/stateCounts"
import { useDebouncedValue } from "@/hooks/useDebouncedValue"
import type { Job, JobSort, JobState, QueueId, StateCounts } from "@/lib/tauri/bindings"
import { filterJobs } from "./filterJobs"
import { JobFilter, type JobFilterScope } from "./JobFilter"
import { JobList } from "./JobList"

export type { JobFilterScope }

/**
 * The Jobs tab of a queue: state chips, filter,
 * list. The route owns the URL; this owns the data.
 *
 *   - **Queue scope**: `filter.name` (job id or name), `filter.error`
 *     (failure reason), `sort` and `pageSize` go to `list_jobs` and run
 *     server-side across the whole state. The inputs stay
 *     instant; the query waits `FILTER_DEBOUNCE_MS` for typing to settle.
 *   - **Cross-queue scope**: first page of every queue, filtered in the
 *     browser (`filterJobs`), no pagination and no sort.
 */
/** Typing stays instant; the server query waits for it to settle. */
export const FILTER_DEBOUNCE_MS = 300
/** Mirrors `job_query::SCAN_CAP`: how many jobs of a state a search looks at. */
const SCAN_CAP = 10_000

export interface JobsPanelProps {
  connectionId: string
  queueId: QueueId
  activeState: JobState
  page: number
  scope: JobFilterScope
  filter: { name: string; error: string }
  sort: JobSort | null
  pageSize: PageSize
  counts: StateCounts | undefined
  onFilterChange: (f: { name: string; error: string }) => void
  onScopeChange: (s: JobFilterScope) => void
  onPageChange: (p: number) => void
  onSortChange: (next: JobSort | null) => void
  onPageSizeChange: (size: PageSize) => void
  /** Open a job in the screen's inspector. */
  onOpenJob?: (job: Job) => void
  /** Shown before the filter (the Failed tab's List / By cause). */
  filterLeading?: ReactNode
}

export function JobsPanel(props: JobsPanelProps) {
  const {
    connectionId,
    queueId,
    activeState,
    page,
    scope,
    filter,
    sort,
    pageSize,
    counts,
    onFilterChange,
    onScopeChange,
    onPageChange,
    onSortChange,
    onPageSizeChange,
    onOpenJob,
    filterLeading,
  } = props

  // Queue scope: search, reason, sort and page size go to the
  // server and run across the whole state. The inputs stay instant (the URL
  // updates per keystroke); the query waits for the typing to settle.
  const serverFilter = useDebouncedValue(filter, FILTER_DEBOUNCE_MS)
  const queueQuery = useJobs({
    connectionId,
    queueId,
    state: activeState,
    page,
    pageSize,
    search: serverFilter.name,
    reason: serverFilter.error,
    sort,
    enabled: scope === "queue",
  })
  // Cross-queue data source (TanStack Query only fires when `useQueues`
  // resolves, per the hook's `enabled` flag).
  const crossQuery = useCrossQueueJobs({
    connectionId,
    state: activeState,
    enabled: scope === "cross",
  })
  const { data: queues } = useQueues(connectionId)
  const jobStates = useJobStates(connectionId)

  const sourcedJobs = useMemo(
    () => (scope === "cross" ? (crossQuery.data?.jobs ?? []) : (queueQuery.data?.jobs ?? [])),
    [scope, crossQuery.data, queueQuery.data],
  )
  const hasFilter = filter.name.trim() !== "" || filter.error.trim() !== ""
  // Cross scope filters the loaded rows; queue scope already got them filtered.
  const filteredJobs = useMemo(
    () => (scope === "cross" ? filterJobs(sourcedJobs, filter) : sourcedJobs),
    [scope, sourcedJobs, filter],
  )
  const matchingCount = Number(queueQuery.data?.total ?? 0)
  // With a server filter `total` is the matching count; the state's size
  // comes from the live counters.
  const stateCount = knownCount(counts, activeState) ?? matchingCount
  useRefreshOnCount(connectionId, queueId, activeState, counts)
  const totalCount =
    scope === "cross"
      ? (crossQuery.data?.aggregateTotal ?? 0)
      : hasFilter
        ? Math.max(stateCount, matchingCount)
        : matchingCount
  const filteredCount = scope === "cross" ? filteredJobs.length : matchingCount
  const queueCount = queues?.length ?? 0
  const scanNote =
    scope === "queue" && queueQuery.data?.scanCapped
      ? `Looked at the newest ${SCAN_CAP.toLocaleString()} ${stateLabel(activeState, jobStates).toLowerCase()} jobs; older ones are not searched or sorted.`
      : null

  const crossQueueNote =
    scope === "cross" && queueCount > 0
      ? `Showing first ${CROSS_QUEUE_PER_QUEUE_LIMIT} of each of ${queueCount} ${queueCount === 1 ? "queue" : "queues"}. Refine with the filter or switch back to single-queue scope for full pagination.`
      : null

  return (
    <>
      <JobFilter
        filter={filter}
        onFilterChange={onFilterChange}
        scope={scope}
        onScopeChange={onScopeChange}
        totalCount={totalCount}
        filteredCount={filteredCount}
        scanNote={scanNote}
        searching={scope === "queue" && queueQuery.isFetching && (hasFilter || sort !== null)}
        showErrorFilter={activeState === "failed" || activeState === "delayed"}
        leading={filterLeading}
      />
      <JobList
        connectionId={connectionId}
        queueId={queueId}
        state={activeState}
        page={page}
        onPageChange={onPageChange}
        jobs={filteredJobs}
        showQueueBadge={scope === "cross"}
        hidePagination={scope === "cross"}
        crossQueueNote={crossQueueNote}
        total={scope === "queue" ? matchingCount : undefined}
        hasMore={scope === "queue" ? (queueQuery.data?.hasMore ?? false) : undefined}
        pageSize={pageSize}
        onPageSizeChange={scope === "queue" ? onPageSizeChange : undefined}
        sort={scope === "queue" ? sort : null}
        onSortChange={scope === "queue" ? onSortChange : undefined}
        filtered={hasFilter}
        loading={scope === "cross" ? crossQuery.isLoading : queueQuery.isLoading}
        errorMessage={
          scope === "queue" && queueQuery.isError
            ? queueQuery.error instanceof Error
              ? queueQuery.error.message
              : "Unknown error"
            : null
        }
        onOpenJob={onOpenJob}
      />
    </>
  )
}

/**
 * The visible state's counter (polled every second) moved: a
 * job arrived or left, so its list is read again now rather than at its
 * next poll. In-flight reads are left to finish.
 */
function useRefreshOnCount(
  connectionId: string,
  queueId: QueueId,
  state: JobState,
  counts: StateCounts | undefined,
) {
  const qc = useQueryClient()
  const count = knownCount(counts, state)
  const seen = useRef<{ key: string; count: number } | null>(null)
  useEffect(() => {
    if (count === undefined) return
    const key = `${connectionId}\u0000${queueId}\u0000${state}`
    const before = seen.current
    seen.current = { key, count }
    if (before?.key === key && before.count !== count) {
      void qc.invalidateQueries(
        { queryKey: jobKeys.listsInState(connectionId, queueId, state) },
        { cancelRefetch: false },
      )
    }
  }, [qc, connectionId, queueId, state, count])
}
