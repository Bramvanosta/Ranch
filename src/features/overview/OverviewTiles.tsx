import { useEffect, useRef, useState } from "react"

import { StatTile } from "@/components/data/charts"
import type { StateTotals } from "@/features/activity/sumCounters"
import { formatCount } from "@/features/metrics/format"
import type { Throughput } from "@/lib/tauri/bindings"

/** Polls kept for the waiting / active / delayed trends (about a minute). */
const HISTORY = 60

/**
 * The five figures of a connection: Waiting, Active
 * and Delayed with how they moved while the screen was open; Failed with
 * how many failed in the last hour; Completed / h. The last two draw the
 * day from BullMQ's own metrics.
 */
export function OverviewTiles({
  connectionId,
  totals,
  lastHour,
  lastDay,
}: {
  connectionId: string
  totals: StateTotals | null
  lastHour: Throughput | undefined
  lastDay: Throughput | undefined
}) {
  const history = useCounterHistory(connectionId, totals)
  const sum = (xs: number[] | undefined) => (xs ?? []).reduce((a, b) => a + b, 0)
  const allOff = (t: Throughput | undefined) =>
    t !== undefined && t.queues.length > 0 && t.metricsOffQueues === t.queues.length
  const failedLastHour = sum(lastHour?.failed)
  const completedLastHour = sum(lastHour?.completed)
  const off = lastHour?.metricsOffQueues ?? 0
  const value = (n: number | undefined) =>
    totals === null || n === undefined ? "—" : formatCount(n)

  return (
    <section
      aria-label="At a glance"
      className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-5"
      data-testid="overview-tiles"
    >
      <StatTile
        label="Waiting"
        value={value(totals ? totals.waiting + totals.prioritized : undefined)}
        sub={
          totals && totals.paused > 0
            ? `+ ${formatCount(totals.paused)} paused`
            : "ready for a worker"
        }
        trend={history.waiting}
        trendColor="var(--state-waiting)"
        testId="overview-tile-waiting"
      />
      <StatTile
        label="Active"
        value={value(totals?.active)}
        sub="being processed"
        trend={history.active}
        trendColor="var(--state-active)"
        testId="overview-tile-active"
      />
      <StatTile
        label="Delayed"
        value={value(totals?.delayed)}
        sub="waiting for their time"
        trend={history.delayed}
        trendColor="var(--state-delayed)"
        testId="overview-tile-delayed"
      />
      <StatTile
        label="Failed"
        value={value(totals?.failed)}
        sub={
          lastHour === undefined || allOff(lastHour)
            ? "kept after their last attempt"
            : failedLastHour > 0
              ? `+${formatCount(failedLastHour)} in the last hour`
              : "none in the last hour"
        }
        tone={totals && totals.failed > 0 ? "danger" : undefined}
        trend={lastDay ? (allOff(lastDay) ? null : lastDay.failed) : undefined}
        trendColor="var(--state-failed)"
        testId="overview-tile-failed"
      />
      <StatTile
        label="Completed / h"
        value={
          lastHour === undefined ? "—" : allOff(lastHour) ? "—" : formatCount(completedLastHour)
        }
        sub={
          lastHour === undefined
            ? "in the last hour"
            : allOff(lastHour)
              ? "metrics off on every queue"
              : off > 0
                ? `${off} of ${lastHour.queues.length} queues record no metrics`
                : "in the last hour"
        }
        trend={lastDay ? (allOff(lastDay) ? null : lastDay.completed) : undefined}
        trendColor="var(--state-completed)"
        testId="overview-tile-completed"
      />
    </section>
  )
}

/** The last polled counters, oldest first; reset when the connection changes. */
function useCounterHistory(connectionId: string, totals: StateTotals | null) {
  const [history, setHistory] = useState<{
    waiting: number[]
    active: number[]
    delayed: number[]
  }>({ waiting: [], active: [], delayed: [] })
  const owner = useRef(connectionId)
  useEffect(() => {
    if (owner.current !== connectionId) {
      owner.current = connectionId
      setHistory({ waiting: [], active: [], delayed: [] })
    }
  }, [connectionId])
  useEffect(() => {
    if (!totals) return
    setHistory((h) => ({
      waiting: [...h.waiting, totals.waiting + totals.prioritized].slice(-HISTORY),
      active: [...h.active, totals.active].slice(-HISTORY),
      delayed: [...h.delayed, totals.delayed].slice(-HISTORY),
    }))
  }, [totals])
  return history
}
