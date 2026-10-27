import { useQueries, useQuery } from "@tanstack/react-query"
import { isMuted, useAttentionMutes, useAttentionReport } from "@/features/attention/useAttention"
import { BACKGROUND_QUEUES_FLOOR_MS, fetchQueues } from "@/features/queues/useQueues"
import { type Connection, commands, type Queue } from "@/lib/tauri/bindings"

/**
 * Every connection and its queue list, read from the `["connections"]` and
 * `["queues", id]` caches the sidebar rows and the home page keep warm.
 * `error` marks a connection whose queues could not be listed (unreachable).
 */
export function useAllConnectionQueues(): {
  connections: Connection[]
  queuesOf: (connectionId: string) => Queue[] | undefined
  unreachable: (connectionId: string) => boolean
} {
  const { data: connections = [] } = useQuery({
    queryKey: ["connections"],
    queryFn: async () => {
      const r = await commands.listConnections()
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
  })
  const results = useQueries({
    queries: connections.map((c) => ({
      queryKey: ["queues", c.id],
      queryFn: () => fetchQueues(c.id),
      // A reader of the caches the sidebar keeps warm: typing in
      // the filter must not send a list_queues per connection.
      staleTime: BACKGROUND_QUEUES_FLOOR_MS,
      retry: false,
    })),
  })
  const byId = new Map(connections.map((c, i) => [c.id, results[i]]))
  return {
    connections,
    queuesOf: (id) => byId.get(id)?.data,
    unreachable: (id) => {
      const r = byId.get(id)
      return Boolean(r?.isError && !r.data)
    },
  }
}

/**
 * The "Needs attention" count of the sidebar:
 * the items of the cross-connection report (failure causes, stuck queues,
 * offline connections) that are not muted.
 */
export function useAttentionCount(): number {
  const { data } = useAttentionReport()
  const { mutes } = useAttentionMutes()
  return (data?.items ?? []).filter((i) => !isMuted(i, mutes)).length
}
