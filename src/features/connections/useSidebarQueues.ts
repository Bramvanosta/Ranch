import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"

import { useRefetchInterval } from "@/features/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { type Connection, commands, type Queue } from "@/lib/tauri/bindings"

/** One `CLIENT LIST` per expanded connection: no need to ask every second. */
const WORKER_COUNTS_FLOOR_MS = 5_000

/**
 * Worker counts of a connection's queues, keyed by queue name;
 * `undefined` while unknown or when this Redis refuses `CLIENT LIST`.
 */
export function useQueueWorkerCounts(
  connectionId: string,
  queues: readonly Queue[] | undefined,
  enabled: boolean,
): ReadonlyMap<string, number> | undefined {
  const refetchInterval = useRefetchInterval(WORKER_COUNTS_FLOOR_MS)
  const refs = (queues ?? []).map((q) => q.queueRef)
  // One self-paced poller per query (see `useSharedPoll`).
  const queryKey = ["queueWorkerCounts", connectionId, refs.map((r) => `${r.prefix}:${r.name}`)]
  useSharedPoll(queryKey, enabled && refs.length > 0 ? refetchInterval : false)
  const { data } = useQuery({
    queryKey,
    queryFn: async () => {
      const r = await commands.countQueueWorkers({ connectionId, queueRefs: refs })
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    enabled: enabled && refs.length > 0,
    refetchInterval: false,
    retry: false,
  })
  if (!data?.supported) return undefined
  return new Map(data.counts.map((c) => [c.queueRef.name, c.workers]))
}

/** Fold / unfold a connection; the row follows at once. */
export function useSetConnectionCollapsed() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (vars: { connectionId: string; collapsed: boolean }) => {
      const r = await commands.setConnectionCollapsed(vars)
      if (r.status === "error") throw new Error(r.error.message)
    },
    onMutate: async ({ connectionId, collapsed }) => {
      await queryClient.cancelQueries({ queryKey: ["connections"] })
      const previous = queryClient.getQueryData<Connection[]>(["connections"])
      queryClient.setQueryData<Connection[]>(["connections"], (old) =>
        old?.map((c) => (c.id === connectionId ? { ...c, sidebarCollapsed: collapsed } : c)),
      )
      return { previous }
    },
    onError: (_err, _vars, ctx) => {
      if (ctx?.previous) queryClient.setQueryData(["connections"], ctx.previous)
      toast.error("Could not save the folded state")
    },
  })
}
