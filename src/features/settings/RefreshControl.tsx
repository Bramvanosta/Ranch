import { useQueryClient } from "@tanstack/react-query"
import { RefreshCwIcon } from "lucide-react"
import { useState } from "react"

import { Button } from "@/components/ui/button"
import { Freshness } from "@/features/freshness/Freshness"
import { QUEUES_REFRESH_FLOOR_MS } from "@/features/queues/useQueues"
import { cn } from "@/lib/utils"

import { useRefetchInterval, useRefreshSettings } from "./useRefreshSettings"

/**
 * Manual refresh for a connection's views: refetches
 * everything on screen that belongs to the connection. When auto-refresh is
 * off, says so — what is on screen is as old as the last click.
 *
 * The icon turns for **this click** only, until what it asked for is back. It
 * used to turn whenever any query of the connection was in flight: with
 * auto-refresh on and a Redis a few tens of milliseconds away, one always is,
 * and the icon never stopped.
 *
 * Next to it, how fresh the connection's counters are (the
 * queue list every connection screen shows): Live, late, or failing.
 */
export function RefreshControl({
  connectionId,
  className,
}: {
  connectionId: string
  className?: string
}) {
  const queryClient = useQueryClient()
  const { settings } = useRefreshSettings()
  const belongs = (queryKey: readonly unknown[]) => queryKey.includes(connectionId)
  const [refreshing, setRefreshing] = useState(false)
  const queuesInterval = useRefetchInterval(QUEUES_REFRESH_FLOOR_MS)

  const refresh = () => {
    setRefreshing(true)
    void queryClient
      .invalidateQueries(
        { predicate: (q) => belongs(q.queryKey), refetchType: "active" },
        // What is on screen is read again; a read already in
        // flight is waited for, not cancelled and sent a second time.
        { cancelRefetch: false },
      )
      // A failed refetch is shown where the data is; here it only ends the turn.
      .catch(() => undefined)
      .finally(() => setRefreshing(false))
  }

  return (
    <span className={cn("inline-flex items-center gap-2", className)}>
      <Freshness queryKey={["queues", connectionId]} intervalMs={queuesInterval} />
      {settings.auto ? null : (
        <span
          className="text-2xs italic text-(--text2)"
          data-testid="auto-refresh-off"
          title="Turn it back on in Settings → Refresh"
        >
          auto-refresh off
        </span>
      )}
      <Button
        type="button"
        size="sm"
        variant="ghost"
        onClick={refresh}
        disabled={refreshing}
        aria-label="Refresh"
        title="Refresh now"
        data-testid="refresh-now"
        className="gap-1.5"
      >
        <RefreshCwIcon
          className={cn("size-3.5", refreshing && "animate-spin")}
          aria-hidden="true"
          data-spinning={refreshing ? "true" : "false"}
        />
        Refresh
      </Button>
    </span>
  )
}
