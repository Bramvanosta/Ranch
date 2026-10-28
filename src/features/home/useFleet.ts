import { useQueries } from "@tanstack/react-query"
import { useEffect, useMemo } from "react"
import { create } from "zustand"
import { useCapabilityLookup } from "@/capabilities"
import { metricsKeys } from "@/data/metrics/keys"
import { fetchThroughput } from "@/data/metrics/queries"
import { queueKeys } from "@/data/queues/keys"
import { fetchQueues, fetchQueueWorkerCounts } from "@/data/queues/queries"
import { serverKeys } from "@/data/server/keys"
import { fetchServerInfo } from "@/data/server/queries"
import { useRefetchInterval } from "@/data/settings/useRefreshSettings"
import { useSharedPolls } from "@/lib/polling/sharedPoll"
import type { Connection, QueueId } from "@/lib/tauri/bindings"
import { type ConnectionHealth, summarizeQueues } from "./homeFilter"
import { HOME_HEALTH_FLOOR_MS } from "./useConnectionHealth"

/** Latency points kept per connection for the health sparkline. */
const LATENCY_POINTS = 30

export interface FleetRow {
  connection: Connection
  health: (ConnectionHealth & { ids: QueueId[] }) | undefined
  /** Why it cannot be reached, as Ranch reports it. */
  error: string | null
  /** When the last failed read happened (ms), for "next try in". */
  errorAt: number
  workers: number | undefined
  failedLastHour: number | undefined
  latencyMs: number | undefined
  latency: number[]
  refetch: () => void
}

interface LatencyStore {
  points: Map<string, number[]>
  add: (id: string, ms: number) => void
}

const useLatencyStore = create<LatencyStore>((set, get) => ({
  points: new Map(),
  add: (id, ms) => {
    const points = new Map(get().points)
    points.set(id, [...(points.get(id) ?? []), ms].slice(-LATENCY_POINTS))
    set({ points })
  },
}))

/**
 * The health of every connection for the home table:
 * its queues and job totals (or why it cannot be reached), its workers, what
 * failed in the last hour, and its latency as it moves. One set of queries
 * per connection, so a dead connection holds nobody else up; never faster
 * than every 10 s.
 */
export function useFleet(connections: readonly Connection[]) {
  const interval = useRefetchInterval(HOME_HEALTH_FLOOR_MS)
  const capabilitiesOf = useCapabilityLookup()
  // Polled by one self-paced loop per query (`useSharedPolls`
  // below), not a timer per observer.
  const common = { refetchInterval: false as const, retry: false }
  const health = useQueries({
    queries: connections.map((c) => ({
      // The connection's queue list, the key the sidebar and the
      // connection's screens read: one poll for all of them.
      queryKey: queueKeys.list(c.id),
      queryFn: () => fetchQueues(c.id),
      select: (queues: Awaited<ReturnType<typeof fetchQueues>>) => ({
        ...summarizeQueues(queues, capabilitiesOf(c.id).jobStates),
        ids: queues.map((q) => q.id),
      }),
      staleTime: HOME_HEALTH_FLOOR_MS,
      ...common,
    })),
  })
  const workers = useQueries({
    queries: connections.map((c, i) => {
      const ids = health[i]?.data?.ids ?? []
      return {
        queryKey: queueKeys.workerCounts(c.id, ids),
        queryFn: () => fetchQueueWorkerCounts(c.id, ids),
        enabled: ids.length > 0,
        ...common,
      }
    }),
  })
  const hour = useQueries({
    queries: connections.map((c, i) => ({
      queryKey: metricsKeys.throughput(c.id, "1h"),
      queryFn: () => fetchThroughput(c.id, "1h"),
      enabled: Boolean(health[i]?.data) && capabilitiesOf(c.id).can("metrics.throughput"),
      ...common,
    })),
  })
  const server = useQueries({
    queries: connections.map((c, i) => ({
      queryKey: serverKeys.info(c.id),
      queryFn: () => fetchServerInfo(c.id),
      enabled: Boolean(health[i]?.data) && capabilitiesOf(c.id).can("server.info"),
      ...common,
    })),
  })

  useSharedPolls(
    connections.flatMap((c, i) => [
      queueKeys.list(c.id),
      queueKeys.workerCounts(c.id, health[i]?.data?.ids ?? []),
      metricsKeys.throughput(c.id, "1h"),
      serverKeys.info(c.id),
    ]),
    interval,
  )

  const addLatency = useLatencyStore((s) => s.add)
  const latency = useLatencyStore((s) => s.points)
  const serverVersion = server.map((s) => s.dataUpdatedAt).join(",")
  // biome-ignore lint/correctness/useExhaustiveDependencies: `serverVersion` stands for `server`.
  useEffect(() => {
    connections.forEach((c, i) => {
      const info = server[i]?.data
      if (info && server[i]?.dataUpdatedAt) addLatency(c.id, info.latencyMs)
    })
  }, [serverVersion])

  const version = [health, workers, hour, server]
    .map((list) => list.map((q) => `${q.dataUpdatedAt}:${q.errorUpdatedAt}`).join(","))
    .join("|")
  // biome-ignore lint/correctness/useExhaustiveDependencies: `version` stands for the queries.
  return useMemo<FleetRow[]>(
    () =>
      connections.map((connection, i) => {
        const h = health[i]
        const w = workers[i]?.data
        const t = hour[i]?.data
        return {
          connection,
          health: h?.data,
          error: h?.error ? (h.error instanceof Error ? h.error.message : String(h.error)) : null,
          errorAt: h?.errorUpdatedAt ?? 0,
          workers: w?.supported ? w.counts.reduce((a, b) => a + b.workers, 0) : undefined,
          failedLastHour: t ? t.failed.reduce((a, b) => a + b, 0) : undefined,
          latencyMs: server[i]?.data?.latencyMs,
          latency: latency.get(connection.id) ?? [],
          refetch: () => void h?.refetch(),
        }
      }),
    [connections, version, latency],
  )
}
