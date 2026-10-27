import { useQueries } from "@tanstack/react-query"
import { useEffect, useMemo } from "react"
import { create } from "zustand"

import { fetchQueues } from "@/features/queues/useQueues"
import { useRefetchInterval } from "@/features/settings/useRefreshSettings"
import { useSharedPolls } from "@/lib/polling/sharedPoll"
import { type Connection, commands, type QueueRef } from "@/lib/tauri/bindings"

import { type ConnectionHealth, summarizeQueues } from "./homeFilter"
import { HOME_HEALTH_FLOOR_MS } from "./useConnectionHealth"

/** Latency points kept per connection for the health sparkline. */
const LATENCY_POINTS = 30

export interface FleetRow {
  connection: Connection
  health: (ConnectionHealth & { refs: QueueRef[] }) | undefined
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
  // Polled by one self-paced loop per query (`useSharedPolls`
  // below), not a timer per observer.
  const common = { refetchInterval: false as const, retry: false }
  const health = useQueries({
    queries: connections.map((c) => ({
      // The connection's queue list, the key the sidebar and the
      // connection's screens read: one poll for all of them.
      queryKey: ["queues", c.id],
      queryFn: () => fetchQueues(c.id),
      select: (queues: Awaited<ReturnType<typeof fetchQueues>>) => ({
        ...summarizeQueues(queues),
        refs: queues.map((q) => q.queueRef),
      }),
      staleTime: HOME_HEALTH_FLOOR_MS,
      ...common,
    })),
  })
  const workers = useQueries({
    queries: connections.map((c, i) => {
      const refs = health[i]?.data?.refs ?? []
      return {
        queryKey: ["queueWorkerCounts", c.id, refs.map((r) => `${r.prefix}:${r.name}`)],
        queryFn: async () => {
          const r = await commands.countQueueWorkers({ connectionId: c.id, queueRefs: refs })
          if (r.status === "error") throw new Error(r.error.message)
          return r.data
        },
        enabled: refs.length > 0,
        ...common,
      }
    }),
  })
  const hour = useQueries({
    queries: connections.map((c, i) => ({
      queryKey: ["throughput", c.id, "1h"],
      queryFn: async () => {
        const r = await commands.getThroughput({ connectionId: c.id, range: "1h" })
        if (r.status === "error") throw new Error(r.error.message)
        return r.data
      },
      enabled: Boolean(health[i]?.data),
      ...common,
    })),
  })
  const server = useQueries({
    queries: connections.map((c, i) => ({
      queryKey: ["serverInfo", c.id],
      queryFn: async () => {
        const r = await commands.getServerInfo({ connectionId: c.id })
        if (r.status === "error") throw new Error(r.error.message)
        return r.data
      },
      enabled: Boolean(health[i]?.data),
      ...common,
    })),
  })

  useSharedPolls(
    connections.flatMap((c, i) => [
      ["queues", c.id],
      [
        "queueWorkerCounts",
        c.id,
        (health[i]?.data?.refs ?? []).map((r) => `${r.prefix}:${r.name}`),
      ],
      ["throughput", c.id, "1h"],
      ["serverInfo", c.id],
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
