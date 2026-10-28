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
import { useJobStates } from "@/capabilities"
import { useQueueMetrics } from "@/data/metrics/queries"
import { useQueues } from "@/data/queues/queries"
import { sumCounters } from "@/domain/sumCounters"
import { RecentJobsCard, StateTotals } from "@/features/activity"
import { StaleNotice } from "@/features/freshness"
import type { MetricsRange, QueueId } from "@/lib/tauri/bindings"
import { OverviewDashboard } from "./OverviewDashboard"

interface QueueOverviewProps {
  connectionId: string
  queueId: QueueId
  /** The window, owned by the route's `?range=` search param. */
  range: MetricsRange
  onRangeChange: (range: MetricsRange) => void
}

export function QueueOverview({ connectionId, queueId, range, onRangeChange }: QueueOverviewProps) {
  const { data, isLoading, isPlaceholderData, error, dataUpdatedAt } = useQueueMetrics(
    connectionId,
    queueId,
    range,
  )
  // The counters come from the list the sidebar already polls: no extra call.
  const queues = useQueues(connectionId).data
  const queue = useMemo(() => queues?.find((q) => q.id === queueId), [queues, queueId])
  const states = useJobStates(connectionId)

  return (
    <div className="flex flex-col gap-4">
      {queue ? (
        <StateTotals
          totals={sumCounters([queue])}
          states={states}
          linkFor={(state, tile) => (
            <Link
              to="/connections/$connectionId/queues/$queueId"
              params={{ connectionId, queueId }}
              search={{ view: "jobs", state, page: 0 }}
              className="block h-full rounded-lg transition-colors hover:[&>div]:border-(--text)/30"
              aria-label={`Open the ${state} jobs`}
            >
              {tile}
            </Link>
          )}
        />
      ) : null}
      <RecentJobsCard connectionId={connectionId} queueId={queueId} />

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
            scopeLabel={queue?.name ?? queueId}
            // Only while another range loads, not on every poll.
            isFetching={isPlaceholderData}
          />
        </>
      )}
    </div>
  )
}
