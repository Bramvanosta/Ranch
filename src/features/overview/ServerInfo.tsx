import { useCapabilities } from "@/capabilities"
import { useMcpSafetySettings } from "@/data/mcp/queries"
import { useServerInfo } from "@/data/server/queries"
import { endpointLine } from "@/domain/bullmqEndpoint"
import { formatBytes, formatCount } from "@/domain/format"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import type { Connection } from "@/lib/tauri/bindings"

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
  // A backend that does not declare it is not asked.
  const { data: info } = useServerInfo(
    connection.id,
    useCapabilities(connection.id).can("server.info"),
  )
  const safety = useMcpSafetySettings().data
  // What the gate will do: read-only or may write, "(default)" when the
  // connection follows the global setting.
  const readonly = connection.mcpReadonly ?? safety?.readonlyDefault
  const writes = safety?.approvalMode === "off" ? "may write" : "may write, with approval"
  const agents =
    readonly === undefined
      ? null
      : `${readonly ? "read-only" : writes}${connection.mcpReadonly === null ? " (default)" : ""}`
  return (
    <>
      {endpointLine(connection) ? <span>{endpointLine(connection)}</span> : null}
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
      {aiOn && agents ? <span data-testid="info-agents">Agents: {agents}</span> : null}
    </>
  )
}
