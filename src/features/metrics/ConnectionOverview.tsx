/**
 * The Overview tab of a connection.
 *
 * Read in five seconds:
 *   1. five stat tiles: Waiting, Active, Delayed (moving while the screen
 *      is open), Failed with the last hour, Completed / h (BullMQ metrics);
 *   2. Needs attention (failure causes, paused queues, queues without a
 *      worker) beside the live events;
 *   3. the busiest queues of the last 6 hours, then the metrics of the
 *      chosen window.
 */
import { useMemo, useState } from "react"

import { sumCounters } from "@/features/activity/sumCounters"
import { useQueueWorkerCounts } from "@/features/connections/useSidebarQueues"
import { StaleNotice } from "@/features/freshness/StaleNotice"
import { BusiestQueues } from "@/features/overview/BusiestQueues"
import { LiveEvents } from "@/features/overview/LiveEvents"
import { NeedsAttention } from "@/features/overview/NeedsAttention"
import { OverviewTiles } from "@/features/overview/OverviewTiles"
import { useQueues } from "@/features/queues/useQueues"
import type { MetricsRange } from "@/lib/tauri/bindings"

import { OverviewDashboard } from "./OverviewDashboard"
import { useConnectionMetrics, useThroughput } from "./useMetrics"

interface ConnectionOverviewProps {
  connectionId: string
  /** The window, owned by the route's `?range=` search param. */
  range: MetricsRange
  onRangeChange: (range: MetricsRange) => void
}

export function ConnectionOverview({
  connectionId,
  range,
  onRangeChange,
}: ConnectionOverviewProps) {
  const { data, isLoading, isPlaceholderData, error, dataUpdatedAt } = useConnectionMetrics(
    connectionId,
    range,
  )
  const queuesQuery = useQueues(connectionId)
  const queues = queuesQuery.data
  const workers = useQueueWorkerCounts(connectionId, queues, true)
  const lastHour = useThroughput(connectionId, "1h").data
  const sixHours = useThroughput(connectionId, "6h").data
  const lastDay = useThroughput(connectionId, "24h").data
  const totals = useMemo(() => (queues ? sumCounters(queues) : null), [queues])
  const [now] = useState(() => Date.now())

  return (
    <div className="flex flex-col gap-5">
      <OverviewTiles
        connectionId={connectionId}
        totals={totals}
        lastHour={lastHour}
        lastDay={lastDay}
      />
      <div className="grid gap-5 xl:grid-cols-2">
        <NeedsAttention
          connectionId={connectionId}
          queues={queues ?? []}
          workers={workers}
          now={now}
        />
        <LiveEvents connectionId={connectionId} />
      </div>
      <BusiestQueues connectionId={connectionId} throughput={sixHours} />

      {/* The error box only when there is nothing to show. */}
      {error && !data ? (
        <div
          role="alert"
          className="rounded-md border border-(--danger)/30 bg-(--danger)/5 p-4 text-sm text-(--state-failed-text)"
          data-testid="connection-overview-error"
        >
          Failed to load connection metrics: {error.message}
        </div>
      ) : isLoading || !data ? (
        <div
          className="rounded-md border border-dashed p-6 text-sm text-(--text2)"
          data-testid="connection-overview-loading"
          role="status"
          aria-label="Loading connection metrics"
        >
          Loading metrics
        </div>
      ) : (
        <>
          {error ? (
            <StaleNotice error={error} updatedAt={dataUpdatedAt} what="the connection metrics" />
          ) : null}
          <OverviewDashboard
            metrics={data}
            connectionId={connectionId}
            range={range}
            onRangeChange={onRangeChange}
            scopeLabel="connection"
            // Only while another range loads, not on every poll.
            isFetching={isPlaceholderData}
          />
        </>
      )}
    </div>
  )
}
