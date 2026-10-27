import { useQuery } from "@tanstack/react-query"

import { formatBytes, formatCount } from "@/features/metrics/format"
import { useRefetchInterval } from "@/features/settings/useRefreshSettings"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { type Connection, commands } from "@/lib/tauri/bindings"

/** INFO is cheap but not free: every 10 s is plenty for an info strip. */
const SERVER_INFO_FLOOR_MS = 10_000

export function useServerInfo(connectionId: string, enabled: boolean) {
  const refetchInterval = useRefetchInterval(SERVER_INFO_FLOOR_MS)
  // One self-paced poller per query, shared (see `useSharedPoll`).
  const queryKey = ["serverInfo", connectionId]
  useSharedPoll(queryKey, refetchInterval)
  return useQuery({
    queryKey,
    queryFn: async () => {
      const r = await commands.getServerInfo({ connectionId })
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    enabled,
    refetchInterval: false,
    retry: false,
  })
}

/**
 * The info strip of a reachable connection: Redis version,
 * latency, memory, ops/s, workers, and what agents may do when agents
 * exist in this runtime.
 */
export function ServerInfoItems({
  connection,
  workers,
}: {
  connection: Connection
  /** Workers attached across the queues; `undefined` = unknown. */
  workers: number | undefined
}) {
  const aiOn = useAiEnabled()
  const { data: info } = useServerInfo(connection.id, true)
  const agents =
    connection.mcpReadonly === true
      ? "read-only"
      : connection.mcpReadonly === false
        ? "may write, with approval"
        : "global default"
  return (
    <>
      <span title={`${connection.host}:${connection.port} (db ${connection.db})`}>
        {connection.host}:{connection.port} · db {connection.db}
      </span>
      {info?.redisVersion ? <span data-testid="info-redis">Redis {info.redisVersion}</span> : null}
      {info ? <span data-testid="info-latency">{info.latencyMs} ms</span> : null}
      {info?.usedMemoryBytes != null ? (
        <span data-testid="info-memory">
          {formatBytes(info.usedMemoryBytes)}
          {info.maxMemoryBytes != null ? ` of ${formatBytes(info.maxMemoryBytes)}` : ""}
        </span>
      ) : null}
      {info?.opsPerSec != null ? (
        <span data-testid="info-ops">{formatCount(info.opsPerSec)} ops/s</span>
      ) : null}
      {workers !== undefined ? (
        <span data-testid="info-workers">
          {formatCount(workers)} {workers === 1 ? "worker" : "workers"}
        </span>
      ) : null}
      {aiOn ? <span data-testid="info-agents">Agents: {agents}</span> : null}
    </>
  )
}
