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
 *
 * A card whose read the backend does not declare is absent: the live events
 * (`events.read`), the busiest queues and the tiles' hourly figures
 * (`metrics.throughput`), the metrics of the window (`metrics.queue`).
 */
import { useMemo, useState } from "react"
import { useCapabilities } from "@/capabilities"
import { useConnectionMetrics, useThroughput } from "@/data/metrics/queries"
import { useQueues, useQueueWorkerCounts } from "@/data/queues/queries"
import { sumCounters } from "@/domain/sumCounters"
import { StaleNotice } from "@/features/freshness"
import { BusiestQueues, LiveEvents, NeedsAttention, OverviewTiles } from "@/features/overview"
import type { MetricsRange } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"
import { OverviewDashboard } from "./OverviewDashboard"

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
  const { can } = useCapabilities(connectionId)
  const metered = can("metrics.queue")
  const throughputOn = can("metrics.throughput")
  const eventsOn = can("events.read")
  const { data, isLoading, isPlaceholderData, error, dataUpdatedAt } = useConnectionMetrics(
    connectionId,
    range,
    { enabled: metered },
  )
  const queuesQuery = useQueues(connectionId)
  const queues = queuesQuery.data
  const workers = useQueueWorkerCounts(connectionId, queues, can("workers.count"))
  const lastHour = useThroughput(connectionId, "1h", { enabled: throughputOn }).data
  const sixHours = useThroughput(connectionId, "6h", { enabled: throughputOn }).data
  const lastDay = useThroughput(connectionId, "24h", { enabled: throughputOn }).data
  const totals = useMemo(() => (queues ? sumCounters(queues) : null), [queues])
  const [now] = useState(() => Date.now())

  return (
    <div className="flex flex-col gap-5">
      <OverviewTiles
        connectionId={connectionId}
        totals={totals}
        lastHour={lastHour}
        lastDay={lastDay}
        throughput={throughputOn}
      />
      <div className={cn("grid gap-5", eventsOn && "xl:grid-cols-2")}>
        <NeedsAttention
          connectionId={connectionId}
          queues={queues ?? []}
          workers={workers}
          now={now}
        />
        {eventsOn ? <LiveEvents connectionId={connectionId} /> : null}
      </div>
      {throughputOn ? <BusiestQueues connectionId={connectionId} throughput={sixHours} /> : null}

      {/* The error box only when there is nothing to show. */}
      {!metered ? null : error && !data ? (
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
