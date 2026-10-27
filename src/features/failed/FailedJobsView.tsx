import { useEffect, useState } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { StaleNotice } from "@/features/freshness/StaleNotice"
import { BulkActionBar } from "@/features/jobs/BulkActionBar"
import { JobList } from "@/features/jobs/JobList"
import { formatCount } from "@/features/metrics/format"
import type { QueueRef } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"
import { useJobSelectionStore } from "@/stores/useJobSelectionStore"

import { FAILED_PAGE_SIZE, type FailedJobsFilter, useFailedJobs } from "./useFailedJobs"

interface FailedJobsViewProps {
  connectionId: string
  filter: FailedJobsFilter
  page: number
  onFilterChange: (filter: FailedJobsFilter) => void
  onPageChange: (page: number) => void
}

const FALLBACK_QUEUE: QueueRef = { prefix: "bull", name: "" }

/**
 * The connection's **Failed** tab: every failed job of
 * every queue in one list, newest first. Added at the product owner's
 * request — BullStudio shows the failed count on its overview but has no list,
 * and going queue by queue is the friction Ranch exists to remove.
 *
 * Reuses the job list (rows open the drill-down, vim navigation, selection)
 * and the bulk action bar, which already handles selections spanning queues.
 */
export function FailedJobsView({
  connectionId,
  filter,
  page,
  onFilterChange,
  onPageChange,
}: FailedJobsViewProps) {
  const { data, isLoading, error, dataUpdatedAt } = useFailedJobs(connectionId, filter, page)
  const clearSelection = useJobSelectionStore((s) => s.clear)

  // The reason field is debounced: each keystroke would otherwise rescan the queues.
  const [reasonDraft, setReasonDraft] = useState(filter.reason)
  useEffect(() => setReasonDraft(filter.reason), [filter.reason])
  useEffect(() => {
    if (reasonDraft === filter.reason) return
    const timer = window.setTimeout(() => onFilterChange({ ...filter, reason: reasonDraft }), 300)
    return () => window.clearTimeout(timer)
  }, [reasonDraft, filter, onFilterChange])

  // A selection made under another filter / connection must not linger.
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset on scope change only
  useEffect(() => clearSelection(), [connectionId, filter.queue, filter.name])

  // The error box only when there is nothing to show.
  if (error && !data) {
    return (
      <div
        role="alert"
        className="rounded-md border border-(--danger)/30 bg-(--danger)/5 p-4 text-sm text-(--state-failed-text)"
        data-testid="failed-jobs-error"
      >
        Failed to load failed jobs: {error.message}
      </div>
    )
  }
  if (isLoading || !data) {
    return (
      <div
        role="status"
        aria-label="Loading failed jobs"
        className="rounded-md border border-dashed p-6 text-sm text-(--text2)"
      >
        Loading failed jobs…
      </div>
    )
  }

  if (data.totalFailed === 0) {
    return (
      <div
        className="rounded-md border border-dashed p-8 text-center text-sm text-(--text2)"
        data-testid="failed-jobs-empty"
      >
        Nothing has failed on this connection. 🎉
      </div>
    )
  }

  const hasFilter = Boolean(filter.queue || filter.name || filter.reason)
  const lastPage = Math.max(0, Math.ceil(data.total / FAILED_PAGE_SIZE) - 1)
  const firstQueue = data.jobs[0]?.jobRef.queue ?? data.queues[0]?.queueRef ?? FALLBACK_QUEUE

  return (
    <div className="flex min-h-0 flex-col gap-3" data-testid="failed-jobs-view">
      {error ? (
        <StaleNotice error={error} updatedAt={dataUpdatedAt} what="the failed jobs" />
      ) : null}
      {/* Per-queue counts double as the queue filter. */}
      <fieldset className="flex flex-wrap items-center gap-1.5">
        <legend className="sr-only">Filter by queue</legend>
        <QueueChip
          label="All queues"
          count={data.totalFailed}
          active={!filter.queue}
          onClick={() => onFilterChange({ ...filter, queue: "" })}
        />
        {data.queues.map((q) => (
          <QueueChip
            key={`${q.queueRef.prefix}:${q.queueRef.name}`}
            label={q.queueRef.name}
            count={q.failed}
            active={filter.queue === q.queueRef.name}
            onClick={() =>
              onFilterChange({
                ...filter,
                queue: filter.queue === q.queueRef.name ? "" : q.queueRef.name,
              })
            }
          />
        ))}
      </fieldset>

      {/* By job type — a way to choose a filter. */}
      {data.types.length > 0 ? (
        <fieldset className="flex flex-wrap gap-1.5">
          <legend className="sr-only">Filter by job type</legend>
          {data.types.map((t) => {
            const active = filter.name === t.name && (!filter.queue || filter.queue === t.queueName)
            return (
              <button
                key={`${t.queueName}:${t.name}`}
                type="button"
                aria-pressed={active}
                title={t.lastReason ?? undefined}
                onClick={() =>
                  onFilterChange(
                    active
                      ? { ...filter, name: "" }
                      : { ...filter, name: t.name, queue: t.queueName },
                  )
                }
                className={cn(
                  "rounded-md border px-2 py-1 text-left text-xs hover:bg-(--surf)/50",
                  active ? "border-(--text)/40 bg-(--surf)" : "border-(--line)",
                )}
                data-testid={`failed-type-${t.queueName}-${t.name}`}
              >
                <span className="font-medium">{t.name || "(unnamed)"}</span>
                <span className="ml-1.5 font-mono text-(--text2)">{t.queueName}</span>
                <span className="ml-1.5 font-tabular text-(--state-failed-text)">
                  {formatCount(t.count)}
                </span>
              </button>
            )
          })}
        </fieldset>
      ) : null}

      <div className="flex flex-wrap items-center gap-2">
        <Input
          value={reasonDraft}
          onChange={(e) => setReasonDraft(e.target.value)}
          placeholder="Filter by failure reason…"
          aria-label="Filter by failure reason"
          className="h-8 max-w-sm text-sm"
        />
        {hasFilter ? (
          <Button
            variant="ghost"
            size="sm"
            className="h-8 text-xs"
            onClick={() => {
              setReasonDraft("")
              onFilterChange({ queue: "", name: "", reason: "" })
            }}
          >
            Clear filters
          </Button>
        ) : null}
        <span className="ml-auto text-xs text-(--text2)" data-testid="failed-jobs-count">
          {hasFilter
            ? `${formatCount(data.total)} matching`
            : `${formatCount(data.totalFailed)} failed`}
        </span>
      </div>

      <BulkActionBar connectionId={connectionId} queueRef={firstQueue} />

      {data.jobs.length === 0 ? (
        <p className="rounded-md border border-dashed p-6 text-center text-sm text-(--text2)">
          No failed job matches these filters.
        </p>
      ) : (
        <JobList
          connectionId={connectionId}
          queueRef={firstQueue}
          state="failed"
          page={page}
          onPageChange={onPageChange}
          jobs={data.jobs}
          showQueueBadge
          hidePagination
          crossQueueNote={
            data.scanCapped
              ? `Looking at the ${formatCount(data.scanned)} most recent failures (up to 500 per queue). Older ones are in each queue's Failed list.`
              : null
          }
        />
      )}

      {data.unreadableQueues.length > 0 ? (
        <p role="note" className="text-xs text-(--state-delayed-text)">
          Could not read: {data.unreadableQueues.join(", ")}.
        </p>
      ) : null}

      {lastPage > 0 ? (
        <div className="flex items-center justify-end gap-2 text-xs">
          <Button
            variant="default"
            size="sm"
            disabled={page === 0}
            onClick={() => onPageChange(page - 1)}
          >
            Newer
          </Button>
          <span className="text-(--text2)">
            {page + 1} / {lastPage + 1}
          </span>
          <Button
            variant="default"
            size="sm"
            disabled={page >= lastPage}
            onClick={() => onPageChange(page + 1)}
          >
            Older
          </Button>
        </div>
      ) : null}
    </div>
  )
}

function QueueChip({
  label,
  count,
  active,
  onClick,
}: {
  label: string
  count: number
  active: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "rounded-full border px-2.5 py-0.5 text-xs hover:bg-(--surf)/50",
        active ? "border-(--text)/40 bg-(--surf) font-medium" : "border-(--line)",
      )}
    >
      <span className="font-mono">{label}</span>
      <span className="ml-1.5 font-tabular text-(--state-failed-text)">{formatCount(count)}</span>
    </button>
  )
}
