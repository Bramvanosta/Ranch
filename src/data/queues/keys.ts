import type { QueueId, QueueMaintenance } from "@/lib/tauri/bindings"

/** Query keys of queues and what is read per queue. */
export const queueKeys = {
  /** Every connection's queue list. */
  all: ["queues"] as const,
  /** A connection's queues with their counters: the most polled key there is. */
  list: (connectionId: string) => [...queueKeys.all, connectionId] as const,
  workerCounts: (connectionId: string, ids: readonly QueueId[]) =>
    ["queueWorkerCounts", connectionId, ids] as const,
  maintenancePreview: (
    connectionId: string,
    queueId: QueueId,
    operation: QueueMaintenance | null,
  ) => ["maintenancePreview", connectionId, queueId, operation] as const,
}
