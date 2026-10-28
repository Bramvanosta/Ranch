import { useQuery } from "@tanstack/react-query"
import { unwrap } from "@/data/result"
import { commands, type QueueId } from "@/lib/tauri/bindings"
import { flowKeys } from "./keys"

/**
 * The flow a job belongs to. Every job has one (a job
 * that is in no flow comes back as a tree of one node), so callers decide
 * what to show from `nodes.length`.
 */
export function useFlow(
  { connectionId, queueId, jobId }: { connectionId: string; queueId: QueueId; jobId: string },
  enabled = true,
) {
  return useQuery({
    queryKey: flowKeys.tree(connectionId, queueId, jobId),
    queryFn: async () => unwrap(await commands.getFlow({ connectionId, queueId, jobId })),
    enabled,
    staleTime: 10_000,
    retry: 1,
  })
}

/** Flow roots found in a queue (bounded discovery). */
export function useFlowRoots(args: { connectionId: string; queueId: QueueId }) {
  return useQuery({
    queryKey: flowKeys.roots(args.connectionId, args.queueId),
    queryFn: () => fetchFlowRoots(args.connectionId, args.queueId),
    staleTime: 10_000,
    retry: 1,
  })
}

/** `list_flow_roots` for `flowKeys.roots`, for a query that reads every queue of a connection. */
export async function fetchFlowRoots(connectionId: string, queueId: QueueId) {
  return unwrap(await commands.listFlowRoots({ connectionId, queueId }))
}
