import { useCallback, useEffect, useMemo, useRef, useState } from "react"

import { DataTable } from "@/components/data/DataTable"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { TooltipProvider } from "@/components/ui/tooltip"
import { StaleNotice } from "@/features/freshness/StaleNotice"
import { useKeyboardShortcut } from "@/hooks/useKeyboardShortcut"
import type { Job, JobSort, JobState, QueueRef } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"
import { jobSelectionKey, useJobSelectionStore } from "@/stores/useJobSelectionStore"
import { JOB_ROW_HEIGHT_PX, JobListRow } from "./JobListRow"
import { jobColumns } from "./jobColumns"
import { useFocusedJobStore } from "./useFocusedJobStore"
import { useJobListNavigation } from "./useJobListNavigation"
import { DEFAULT_PAGE_SIZE, PAGE_SIZES, type PageSize, useJobs } from "./useJobs"

/**
 * Virtualized paginated job list.
 *
 * Two operating modes:
 *
 *   1. **Self-managed (default).** When the `jobs` prop is
 *      omitted, the component owns the data fetch via `useJobs`,
 *      renders the pagination footer, and handles loading/error states
 *      internally.
 *
 *   2. **Externally-sourced.** When `jobs` is provided, the
 *      route owns the data (either via `useJobs` + client-side filter
 *      OR via `useCrossQueueJobs`) and `JobList` becomes a pure
 *      renderer. Pagination can be hidden via `hidePagination`, and a
 *      one-line `crossQueueNote` rendered under the list. Each row
 *      can show a small queue badge via `showQueueBadge`.
 *      The route passes the server's `total` / `hasMore` so
 *      the footer pages over the whole (possibly filtered) state, plus
 *      the sort and page-size controls.
 *
 * Both modes share the virtualization (`@tanstack/react-virtual`) so
 * even 100k jobs only mount ~10–15 DOM rows at any scroll position.
 */
interface JobListProps {
  connectionId: string
  queueRef: QueueRef
  state: JobState
  page: number
  onPageChange: (page: number) => void
  /** override-mode jobs. When set, internal `useJobs` is bypassed. */
  jobs?: Job[]
  /** Show a small `[queue]` badge before each row's job ID. */
  showQueueBadge?: boolean
  /** Hide the Previous/Next footer (used in cross-queue scope). */
  hidePagination?: boolean
  /** Small italic note rendered under the list. */
  crossQueueNote?: string | null
  /** Server totals for externally-sourced jobs (else `jobs.length`). */
  total?: number
  hasMore?: boolean
  pageSize?: PageSize
  onPageSizeChange?: (size: PageSize) => void
  /** server-side sort; the header cells become buttons when `onSortChange` is set. */
  sort?: JobSort | null
  onSortChange?: (next: JobSort | null) => void
  /** A filter is active: the empty state says so. */
  filtered?: boolean
  /** The route's own loading / error state (externally-sourced mode). */
  loading?: boolean
  errorMessage?: string | null
  /** Open a job beside the list (the screen's inspector) instead of on its page. */
  onOpenJob?: (job: Job) => void
}

export function JobList({
  connectionId,
  queueRef,
  state,
  page,
  onPageChange,
  jobs: externalJobs,
  showQueueBadge = false,
  hidePagination = false,
  crossQueueNote = null,
  total: externalTotal,
  hasMore: externalHasMore,
  pageSize = DEFAULT_PAGE_SIZE,
  onPageSizeChange,
  sort = null,
  onSortChange,
  filtered = false,
  loading = false,
  errorMessage = null,
  onOpenJob,
}: JobListProps) {
  const externallySourced = externalJobs !== undefined

  // Self-managed path: always call useJobs (hooks rule), then ignore the
  // result when the parent provided `jobs` explicitly. The query stays
  // cached so toggling back to self-managed mode is instant.
  const {
    data,
    isLoading: queryIsLoading,
    isError: queryIsError,
    error: queryError,
    isFetching,
    isPlaceholderData,
    dataUpdatedAt,
  } = useJobs({
    connectionId,
    queueRef,
    state,
    page,
    pageSize,
    sort,
    // The parent's query carries its own search / sort: fetching
    // the unfiltered page here as well would be a wasted round trip.
    enabled: !externallySourced,
  })

  const jobs = externallySourced ? (externalJobs ?? []) : (data?.jobs ?? [])
  const total = externallySourced ? (externalTotal ?? jobs.length) : (data?.total ?? 0)
  const hasMore = externallySourced ? (externalHasMore ?? false) : (data?.hasMore ?? false)

  // Self-managed mode reads the internal query; otherwise the parent
  // reports its own state through `loading` / `errorMessage`.
  const isLoading = externallySourced ? loading : queryIsLoading
  const isError = externallySourced ? errorMessage !== null : queryIsError
  const error = externallySourced ? new Error(errorMessage ?? "Unknown error") : queryError

  // Snapshot "now" once per data refresh so age cells don't jitter.
  // In self-managed mode `data` is the trigger; in externally-sourced
  // mode the parent's `externalJobs` reference change is the trigger.
  // The combined dep keeps the single effect honest under both paths.
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (externallySourced) {
      if (externalJobs) setNow(Date.now())
    } else if (data) {
      setNow(Date.now())
    }
  }, [data, externalJobs, externallySourced])

  // While another page, state or sort loads, the previous rows
  // stay (same queue only), dimmed, under the page they belong to.
  const placeholder = !externallySourced && isPlaceholderData
  const shownPage = useRef(page)
  if (!placeholder) shownPage.current = page
  const totalPages = total > 0 ? Math.ceil(total / pageSize) : 1
  const currentPageHumanIdx = shownPage.current + 1

  if (isLoading) {
    return <JobListSkeleton />
  }

  // The error box only when there is nothing to show; jobs
  // already on screen stay, with a notice.
  if (isError && jobs.length === 0) {
    return (
      <div
        role="alert"
        className="flex flex-col gap-1 rounded-md border border-(--danger)/40 bg-(--danger)/10 p-3 text-sm text-(--state-failed-text)"
      >
        <p className="font-medium">Could not list jobs</p>
        <p className="text-xs">{error instanceof Error ? error.message : "Unknown error"}</p>
      </div>
    )
  }

  if (jobs.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 rounded-md border border-dashed border-(--line) bg-(--surf)/20 p-8 text-center">
        <p className="text-sm font-medium">
          {filtered ? `No ${state} jobs match the filter.` : `No ${state} jobs in this queue.`}
        </p>
        <p className="text-xs text-(--text2)">
          {filtered
            ? "The filter looked at every job of this state, not just one page."
            : "Try a different state above — counts come straight from the queue's live counters."}
        </p>
      </div>
    )
  }

  return (
    <TooltipProvider>
      <div
        className="flex min-h-0 flex-1 flex-col gap-2"
        data-testid="job-list"
        data-fetching={isFetching ? "true" : "false"}
        data-placeholder={placeholder ? "true" : "false"}
        aria-busy={placeholder}
      >
        <div
          role="progressbar"
          aria-label="Loading jobs"
          aria-hidden={!placeholder}
          className={cn(
            "h-0.5 w-full rounded-full bg-(--accent) transition-opacity",
            placeholder ? "animate-pulse opacity-100" : "opacity-0",
          )}
        />
        {isError ? (
          <StaleNotice
            error={error}
            updatedAt={externallySourced ? 0 : dataUpdatedAt}
            what="the jobs"
          />
        ) : null}
        <JobListVirtual
          dimmed={placeholder}
          jobs={jobs}
          now={now}
          connectionId={connectionId}
          showQueueBadge={showQueueBadge}
          listState={state}
          sort={sort}
          onSortChange={onSortChange}
          onOpenJob={onOpenJob}
        />
        <JobListBulkBindings jobs={jobs} />
        {/* vim-style keyboard navigation hook (j/k, arrows,
            Enter, cmd-R/Backspace/P). Mounted as a sibling so its
            keydown listener lives at list-level, not inside the
            virtualizer (which only mounts ~10 rows). The hook owns
            the focusedIndex state + passes it through a small
            context to JobListRow for the highlight ring. */}
        <JobListKeyboardNavigation jobs={jobs} connectionId={connectionId} onOpen={onOpenJob} />
        {hidePagination ? null : (
          <div className="flex items-center justify-between gap-2 border-t border-(--line)/40 px-3 py-2 text-xs text-(--text2)">
            <span>
              Page {currentPageHumanIdx} of {totalPages} ({total.toLocaleString()} total)
            </span>
            <div className="flex items-center gap-1">
              {onPageSizeChange ? (
                <label className="mr-2 flex items-center gap-1.5">
                  <span>Rows</span>
                  <select
                    value={pageSize}
                    onChange={(e) => onPageSizeChange(Number(e.target.value) as PageSize)}
                    data-testid="job-list-page-size"
                    className="h-7 rounded-md border border-(--line) bg-transparent px-1.5 text-xs text-(--text) dark:bg-(--field)/30"
                  >
                    {PAGE_SIZES.map((size) => (
                      <option key={size} value={size}>
                        {size}
                      </option>
                    ))}
                  </select>
                </label>
              ) : null}
              <Button
                variant="ghost"
                size="sm"
                disabled={page === 0}
                onClick={() => onPageChange(Math.max(0, page - 1))}
              >
                Previous
              </Button>
              <Button
                variant="ghost"
                size="sm"
                disabled={!hasMore}
                onClick={() => onPageChange(page + 1)}
              >
                Next
              </Button>
            </div>
          </div>
        )}
        {crossQueueNote ? (
          <p
            className="px-3 text-2xs italic text-(--text2)"
            data-testid="job-list-cross-queue-note"
          >
            {crossQueueNote}
          </p>
        ) : null}
      </div>
    </TooltipProvider>
  )
}

function JobListVirtual({
  dimmed = false,
  jobs,
  now,
  connectionId,
  showQueueBadge,
  listState,
  sort,
  onSortChange,
  onOpenJob,
}: {
  /** The rows are a placeholder while other ones load. */
  dimmed?: boolean
  jobs: Job[]
  now: number
  connectionId: string
  showQueueBadge: boolean
  listState: JobState
  sort: JobSort | null
  onSortChange?: (next: JobSort | null) => void
  onOpenJob?: (job: Job) => void
}) {
  // last-clicked index tracker for shift-click range
  // selection. Reset to null when the list contents change (caller
  // resets via `key` prop / effect; not modeled here for simplicity).
  const lastClickedRef = useRef<number | null>(null)
  const addRange = useJobSelectionStore((s) => s.addRange)
  const removeRange = useJobSelectionStore((s) => s.removeRange)
  const selection = useJobSelectionStore((s) => s.selection)

  // The header's select-all checkbox works on the page on screen — the same
  // scope as cmd-A, and it leaves a selection made on other pages alone.
  const pageKeys = useMemo(
    () => jobs.map((j) => jobSelectionKey({ jobRef: j.jobRef, state: j.state })),
    [jobs],
  )
  const selectedOnPage = pageKeys.filter((k) => selection.has(k)).length
  const pageSelection =
    selectedOnPage === 0 ? "none" : selectedOnPage === pageKeys.length ? "all" : "some"
  const togglePage = useCallback(() => {
    if (pageSelection === "all") removeRange(pageKeys)
    else addRange(jobs.map((j) => ({ jobRef: j.jobRef, state: j.state })))
  }, [pageSelection, pageKeys, jobs, addRange, removeRange])

  const onClickCheckbox = useCallback((index: number) => {
    lastClickedRef.current = index
  }, [])

  const onShiftClickCheckbox = useCallback(
    (index: number) => {
      const last = lastClickedRef.current
      if (last === null) {
        // No prior click — treat as plain click.
        lastClickedRef.current = index
        const job = jobs[index]
        if (job) addRange([{ jobRef: job.jobRef, state: job.state }])
        return
      }
      const [a, b] = last < index ? [last, index] : [index, last]
      const entries = jobs.slice(a, b + 1).map((j) => ({
        jobRef: j.jobRef,
        state: j.state,
      }))
      addRange(entries)
      lastClickedRef.current = index
    },
    [jobs, addRange],
  )

  // The list on the shared DataTable, with the columns
  // of its state: the header is drawn from them, and each row renders them.
  const columns = useMemo(
    () =>
      jobColumns({
        state: listState,
        showQueue: showQueueBadge,
        sort,
        onSortChange,
        pageSelection: jobs.length > 0 ? pageSelection : undefined,
        onTogglePage: togglePage,
      }),
    [listState, showQueueBadge, sort, onSortChange, jobs.length, pageSelection, togglePage],
  )
  const focusedIndex = useFocusedJobStore((s) => s.focusedIndex)

  return (
    <DataTable
      rows={jobs}
      columns={columns}
      rowKey={(job) => `${job.jobRef.queue.name}:${job.jobRef.id}`}
      rowHeight={JOB_ROW_HEIGHT_PX}
      ariaLabel="Jobs"
      testId="job-list-virtual"
      className={cn("transition-opacity", dimmed && "opacity-50")}
      focusedIndex={focusedIndex}
      renderRow={(job, index) => (
        <JobListRow
          job={job}
          now={now}
          connectionId={connectionId}
          columns={columns}
          index={index}
          onClickCheckbox={onClickCheckbox}
          onShiftClickCheckbox={onShiftClickCheckbox}
          onOpen={onOpenJob}
        />
      )}
    />
  )
}

/**
 * Owns the JobList route's keyboard bindings for bulk
 * selection. Mounted as a sibling so the keyboard hook lives at the
 * list-level, not inside the virtualizer (which only mounts ~10
 * rows; binding cmd-A on a row would leak).
 *
 * `cmd-A` replaces the selection with every job in `jobs` (the
 * currently-visible / filtered page). NOT the whole queue — that's
 * an explicit decision.
 */
function JobListBulkBindings({ jobs }: { jobs: Job[] }) {
  const selectAll = useJobSelectionStore((s) => s.selectAll)
  useKeyboardShortcut("cmd+a", (e) => {
    // Skip when typing in an input (filter, etc.). The hook itself
    // doesn't check focus, so we add a small guard here.
    const target = e.target as HTMLElement | null
    const tag = target?.tagName?.toLowerCase()
    if (tag === "input" || tag === "textarea" || target?.isContentEditable) {
      return
    }
    e.preventDefault()
    selectAll(jobs.map((j) => ({ jobRef: j.jobRef, state: j.state })))
  })
  return null
}

/**
 * Mounts `useJobListNavigation` to own the vim-style
 * keyboard bindings (j/k/arrows/Enter/cmd-R/cmd-Backspace/cmd-P).
 * Returns null — the hook only attaches a window-level keydown
 * listener. The focused-row highlight is applied via the
 * `useFocusedJobIndex` store read inside `JobListRow`.
 */
function JobListKeyboardNavigation({
  jobs,
  connectionId,
  onOpen,
}: {
  jobs: Job[]
  connectionId: string
  onOpen?: (job: Job) => void
}) {
  // We expose the focused index via a tiny module-level store so
  // JobListRow can read it without prop-drilling through the
  // virtualizer (which already takes many props).
  const setFocused = useFocusedJobStore((s) => s.setFocusedIndex)
  useJobListNavigation({
    jobs,
    connectionId,
    onFocusChange: setFocused,
    onOpen,
  })
  // A new list starts unmarked: the mark belongs to whoever used the keyboard here.
  useEffect(() => () => setFocused(null), [setFocused])
  return null
}

function JobListSkeleton() {
  return (
    <div
      role="status"
      aria-label="Loading jobs"
      className="flex flex-col gap-1 rounded-md border border-(--line)/40 p-2"
    >
      {[0, 1, 2, 3, 4].map((i) => (
        <div key={i} className="flex items-center gap-3 px-1 py-2">
          <Skeleton className="size-4 shrink-0" />
          <Skeleton className="h-4 w-12 shrink-0" />
          <Skeleton className="h-4 flex-1" />
          <Skeleton className="h-4 w-12 shrink-0" />
          <Skeleton className="h-5 w-16 shrink-0" />
        </div>
      ))}
    </div>
  )
}
