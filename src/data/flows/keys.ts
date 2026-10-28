import type { QueueId } from "@/lib/tauri/bindings"

/** Query keys of flows: a job's tree, and the flow roots of a queue. */
export const flowKeys = {
  /** Every flow tree open on a connection. */
  trees: (connectionId: string) => ["flow", connectionId] as const,
  tree: (connectionId: string, queueId: QueueId, jobId: string) =>
    [...flowKeys.trees(connectionId), queueId, jobId] as const,
  /** Every queue's flow roots on a connection. */
  allRoots: (connectionId: string) => ["flowRoots", connectionId] as const,
  roots: (connectionId: string, queueId: QueueId) =>
    [...flowKeys.allRoots(connectionId), queueId] as const,
}
