import type { QueueId } from "@/lib/tauri/bindings"

/** Query keys of the workers attached to queues. */
export const workerKeys = {
  list: (connectionId: string, queueId: QueueId) => ["workers", connectionId, queueId] as const,
}
