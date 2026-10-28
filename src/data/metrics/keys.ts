import type { MetricsRange, QueueId } from "@/lib/tauri/bindings"

/** Query keys of metrics: per queue, per connection, and throughput over time. */
export const metricsKeys = {
  queue: (connectionId: string, queueId: QueueId | undefined, range: MetricsRange) =>
    ["queue-metrics", connectionId, queueId ?? "", range] as const,
  connection: (connectionId: string, range: MetricsRange) =>
    ["connection-metrics", connectionId, range] as const,
  throughput: (connectionId: string, range: MetricsRange) =>
    ["throughput", connectionId, range] as const,
}
