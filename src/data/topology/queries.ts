import { useQuery } from "@tanstack/react-query"
import { unwrap } from "@/data/result"
import { useRefetchInterval } from "@/data/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { commands } from "@/lib/tauri/bindings"
import { topologyKeys } from "./keys"

/**
 * Which queues feed which workers, and the flow links between queues
 * (`get_topology`). It reads every queue: polled through the refresh
 * setting, never faster than `floorMs`.
 */
export function useTopology(connectionId: string, floorMs: number) {
  const queryKey = topologyKeys.of(connectionId)
  // One self-paced poller per query, shared (see `useSharedPoll`).
  useSharedPoll(queryKey, useRefetchInterval(floorMs))
  return useQuery({
    queryKey,
    queryFn: async () => {
      return unwrap(await commands.getTopology({ connectionId }))
    },
    refetchInterval: false,
  })
}
