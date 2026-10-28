import { useQueries } from "@tanstack/react-query"
import { isMuted, useAttentionMutes, useAttentionReport } from "@/data/attention/queries"
import { useConnections } from "@/data/connections/queries"
import { queueKeys } from "@/data/queues/keys"
import { BACKGROUND_QUEUES_FLOOR_MS, fetchQueues } from "@/data/queues/queries"
import type { Connection, Queue } from "@/lib/tauri/bindings"

/**
 * Every connection and its queue list, read from the `["connections"]` and
 * `queueKeys.list(id)` caches the sidebar rows and the home page keep warm.
 * `error` marks a connection whose queues could not be listed (unreachable).
 */
export function useAllConnectionQueues(): {
  connections: Connection[]
  queuesOf: (connectionId: string) => Queue[] | undefined
  unreachable: (connectionId: string) => boolean
} {
  const { data: connections = [] } = useConnections()
  const results = useQueries({
    queries: connections.map((c) => ({
      queryKey: queueKeys.list(c.id),
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
