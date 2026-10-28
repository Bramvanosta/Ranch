/**
 * TanStack Query hooks for the overview dashboard.
 *
 * `useQueueMetrics` powers the per-queue Overview tab; aggregates the
 * 24h ZSET counts + the bounded HMGET sample for processing time /
 * queue delay (backend caps at 100 sampled jobs per call).
 *
 * `useConnectionMetrics` is the env-level aggregator: same shape but
 * computed across every queue on the connection (weighted average).
 *
 * Refetch every 5s. The backend pipelines 48 ZCOUNTs + 100 HMGETs in a
 * single Redis round-trip; 5s keeps the dashboard fresh without
 * doubling our op budget vs. the 1s queue-list poll.
 */
import { useQuery } from "@tanstack/react-query"
import { errorKind, unwrap } from "@/data/result"
import { useRefetchInterval } from "@/data/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import type { MetricsRange, QueueId } from "@/lib/tauri/bindings"
import { commands } from "@/lib/tauri/bindings"
import { metricsKeys } from "./keys"

const METRICS_REFETCH_MS = 5_000

export function useQueueMetrics(
  connectionId: string,
  queueId: QueueId | undefined,
  range: MetricsRange = "24h",
) {
  const refetchInterval = useRefetchInterval(METRICS_REFETCH_MS)
  // One self-paced poller per query, shared (see `useSharedPoll`).
  const queryKey = metricsKeys.queue(connectionId, queueId, range)
  useSharedPoll(queryKey, refetchInterval)
  return useQuery({
    queryKey,
    queryFn: async () => {
      if (!queueId) throw new Error("queueId required")
      return unwrap(await commands.getQueueMetrics({ connectionId, queueId, range, nowMs: null }))
    },
    enabled: Boolean(queueId),
    // Keep the previous window on screen while the new one loads.
    placeholderData: (previous) => previous,
    refetchInterval: false,
    retry: (failureCount, error) => {
      const kind = errorKind(error)
      if (kind === "schemaNotRecognized") return false
      return failureCount < 2
    },
  })
}

export function useConnectionMetrics(
  connectionId: string,
  range: MetricsRange = "24h",
  { enabled = true }: { enabled?: boolean } = {},
) {
  const refetchInterval = useRefetchInterval(METRICS_REFETCH_MS)
  // One self-paced poller per query, shared (see `useSharedPoll`).
  const queryKey = metricsKeys.connection(connectionId, range)
  useSharedPoll(queryKey, enabled ? refetchInterval : false)
  return useQuery({
    queryKey,
    enabled,
    placeholderData: (previous) => previous,
    queryFn: async () => {
      return unwrap(await commands.getConnectionMetrics({ connectionId, range }))
    },
    refetchInterval: false,
    retry: (failureCount, error) => {
      const kind = errorKind(error)
      if (kind === "schemaNotRecognized") return false
      return failureCount < 2
    },
  })
}

/**
 * Completed / failed over time for every queue of a connection
 * (native metrics; `metricsOff` where a queue records none). A screen that
 * reads it at a slower pace than the dashboard (the Map, the Timeline)
 * passes its own floor; the shared poll runs as often as its most eager
 * reader asks.
 */
export function useThroughput(
  connectionId: string,
  range: MetricsRange = "6h",
  { floorMs = METRICS_REFETCH_MS, enabled = true }: { floorMs?: number; enabled?: boolean } = {},
) {
  const refetchInterval = useRefetchInterval(floorMs)
  // One self-paced poller per query, shared (see `useSharedPoll`).
  const queryKey = metricsKeys.throughput(connectionId, range)
  useSharedPoll(queryKey, enabled ? refetchInterval : false)
  return useQuery({
    queryKey,
    queryFn: async () => {
      return unwrap(await commands.getThroughput({ connectionId, range }))
    },
    enabled: enabled && connectionId !== "",
    placeholderData: (previous) => previous,
    refetchInterval: false,
  })
}

/** `get_throughput` for a `metricsKeys.throughput` query. */
export async function fetchThroughput(connectionId: string, range: MetricsRange) {
  return unwrap(await commands.getThroughput({ connectionId, range }))
}
