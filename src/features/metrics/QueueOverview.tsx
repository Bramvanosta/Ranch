/**
 * Per-queue Overview tab.
 *
 * On top: the queue's jobs per state (live counters; each tile opens that
 * state in the Jobs tab) and what ran last on it.
 *
 * Mounts the `getQueueMetrics` query and feeds the raw `QueueMetrics`
 * payload to `OverviewDashboard`. Polls every 5s while the document is
 * visible. Empty/loading/error states are rendered inline so the
 * dashboard stays at a stable height across refetches.
 */
import { Link } from "@tanstack/react-router"
import { useMemo } from "react"

import { RecentJobsCard } from "@/features/activity/RecentJobsCard"
import { StateTotals } from "@/features/activity/StateTotals"
import { sumCounters } from "@/features/activity/sumCounters"
import { StaleNotice } from "@/features/freshness/StaleNotice"
import { useQueues } from "@/features/queues/useQueues"
import type { MetricsRange, QueueRef } from "@/lib/tauri/bindings"

import { OverviewDashboard } from "./OverviewDashboard"
import { useQueueMetrics } from "./useMetrics"

interface QueueOverviewProps {
  connectionId: string
  queueRef: QueueRef
  /** The window, owned by the route's `?range=` search param. */
  range: MetricsRange
  onRangeChange: (range: MetricsRange) => void
}

export function QueueOverview({
  connectionId,
  queueRef,
  range,
  onRangeChange,
}: QueueOverviewProps) {
  const { data, isLoading, isPlaceholderData, error, dataUpdatedAt } = useQueueMetrics(
    connectionId,
    queueRef,
    range,
  )
  // The counters come from the list the sidebar already polls: no extra call.
  const queues = useQueues(connectionId).data
  const queue = useMemo(
    () =>
      queues?.find(
        (q) => q.queueRef.name === queueRef.name && q.queueRef.prefix === queueRef.prefix,
      ),
    [queues, queueRef.name, queueRef.prefix],
  )

  return (
    <div className="flex flex-col gap-4">
      {queue ? (
        <StateTotals
          totals={sumCounters([queue])}
          linkFor={(state, tile) => (
            <Link
              to="/connections/$connectionId/queues/$queueName"
              params={{ connectionId, queueName: queueRef.name }}
              search={{ view: "jobs", state, page: 0 }}
              className="block h-full rounded-lg transition-colors hover:[&>div]:border-(--text)/30"
              aria-label={`Open the ${state} jobs`}
            >
              {tile}
            </Link>
          )}
        />
      ) : null}
      <RecentJobsCard connectionId={connectionId} queueRef={queueRef} />

      {/* The error box only when there is nothing to show. */}
      {error && !data ? (
        <div
          role="alert"
          className="rounded-md border border-(--danger)/30 bg-(--danger)/5 p-4 text-sm text-(--state-failed-text)"
          data-testid="queue-overview-error"
        >
          Failed to load queue metrics: {error.message}
        </div>
      ) : isLoading || !data ? (
        <div
          className="rounded-md border border-dashed p-6 text-sm text-(--text2)"
          data-testid="queue-overview-loading"
          role="status"
          aria-label="Loading queue metrics"
        >
          Loading metrics
        </div>
      ) : (
        <>
          {error ? (
            <StaleNotice error={error} updatedAt={dataUpdatedAt} what="the queue metrics" />
          ) : null}
          <OverviewDashboard
            metrics={data}
            connectionId={connectionId}
            range={range}
            onRangeChange={onRangeChange}
            scopeLabel={queueRef.name}
            // Only while another range loads, not on every poll.
            isFetching={isPlaceholderData}
          />
        </>
      )}
    </div>
  )
}
