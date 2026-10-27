/**
 * TanStack Query hooks for the overview dashboard.
 *
 * `useQueueMetrics` powers the per-queue Overview tab; aggregates the
 * 24h ZSET counts + the bounded HMGET sample for processing time /
 * queue delay (backend caps at 100 sampled jobs per call).
 *
 * `useConnectionMetrics` is the env-level aggregator — same shape but
 * computed across every queue on the connection (weighted average).
 *
 * Refetch every 5s. The backend pipelines 48 ZCOUNTs + 100 HMGETs in a
 * single Redis round-trip; 5s keeps the dashboard fresh without
 * doubling our op budget vs. the 1s queue-list poll.
 */
import { useQuery } from "@tanstack/react-query"
import { useRefetchInterval } from "@/features/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import type { MetricsRange, QueueRef } from "@/lib/tauri/bindings"
import { commands } from "@/lib/tauri/bindings"

const METRICS_REFETCH_MS = 5_000

export function useQueueMetrics(
  connectionId: string,
  queueRef: QueueRef | undefined,
  range: MetricsRange = "24h",
) {
  const refetchInterval = useRefetchInterval(METRICS_REFETCH_MS)
  // One self-paced poller per query, shared (see `useSharedPoll`).
  const queryKey = [
    "queue-metrics",
    connectionId,
    queueRef?.prefix ?? "",
    queueRef?.name ?? "",
    range,
  ]
  useSharedPoll(queryKey, refetchInterval)
  return useQuery({
    queryKey,
    queryFn: async () => {
      if (!queueRef) throw new Error("queueRef required")
      const result = await commands.getQueueMetrics({ connectionId, queueRef, range, nowMs: null })
      if (result.status === "error") {
        const err = new Error(result.error.message)
        ;(err as Error & { kind?: string }).kind = result.error.kind
        throw err
      }
      return result.data
    },
    enabled: Boolean(queueRef?.name),
    // Keep the previous window on screen while the new one loads.
    placeholderData: (previous) => previous,
    refetchInterval: false,
    retry: (failureCount, error) => {
      const kind = (error as Error & { kind?: string }).kind
      if (kind === "schemaNotRecognized") return false
      return failureCount < 2
    },
  })
}

export function useConnectionMetrics(connectionId: string, range: MetricsRange = "24h") {
  const refetchInterval = useRefetchInterval(METRICS_REFETCH_MS)
  // One self-paced poller per query, shared (see `useSharedPoll`).
  const queryKey = ["connection-metrics", connectionId, range]
  useSharedPoll(queryKey, refetchInterval)
  return useQuery({
    queryKey,
    placeholderData: (previous) => previous,
    queryFn: async () => {
      const result = await commands.getConnectionMetrics({ connectionId, range })
      if (result.status === "error") {
        const err = new Error(result.error.message)
        ;(err as Error & { kind?: string }).kind = result.error.kind
        throw err
      }
      return result.data
    },
    refetchInterval: false,
    retry: (failureCount, error) => {
      const kind = (error as Error & { kind?: string }).kind
      if (kind === "schemaNotRecognized") return false
      return failureCount < 2
    },
  })
}

/**
 * Completed / failed over time for every queue of a connection
 * (native metrics; `metricsOff` where a queue records none).
 */
export function useThroughput(connectionId: string, range: MetricsRange = "6h") {
  const refetchInterval = useRefetchInterval(METRICS_REFETCH_MS)
  // One self-paced poller per query, shared (see `useSharedPoll`).
  const queryKey = ["throughput", connectionId, range]
  useSharedPoll(queryKey, refetchInterval)
  return useQuery({
    queryKey,
    queryFn: async () => {
      const result = await commands.getThroughput({ connectionId, range })
      if (result.status === "error") throw new Error(result.error.message)
      return result.data
    },
    enabled: connectionId !== "",
    placeholderData: (previous) => previous,
    refetchInterval: false,
  })
}
