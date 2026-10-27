import { useQuery } from "@tanstack/react-query"

import { useRefetchInterval } from "@/features/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { commands, type QueueRef } from "@/lib/tauri/bindings"

/**
 * The BullMQ workers attached to a queue.
 *
 * `list_workers` reads `CLIENT LIST`; a Redis that forbids it answers
 * `supported: false` (not an error — the actor probes once per connection).
 * Workers come and go and `idleSecs` moves every second, so the list
 * refreshes on its own every `WORKERS_REFRESH_MS`; the tab also has a manual
 * refresh. Once unsupported, polling stops: the answer cannot change until
 * the connection is re-opened.
 */
export const WORKERS_REFRESH_MS = 5_000

export interface UseWorkersArgs {
  connectionId: string
  queueRef: QueueRef
}

export function useWorkers(args: UseWorkersArgs) {
  // Follows Settings → Refresh, never faster than 5 s.
  const interval = useRefetchInterval(WORKERS_REFRESH_MS)
  const queryKey = ["workers", args.connectionId, args.queueRef.prefix, args.queueRef.name]
  const query = useQuery({
    queryKey,
    queryFn: async () => {
      const result = await commands.listWorkers({
        connectionId: args.connectionId,
        queueRef: args.queueRef,
      })
      if (result.status === "error") {
        const err = new Error(result.error.message)
        ;(err as Error & { kind?: string }).kind = result.error.kind
        throw err
      }
      return result.data
    },
    staleTime: WORKERS_REFRESH_MS,
    refetchInterval: false,
    retry: 1,
  })
  // One self-paced poller per query (see `useSharedPoll`).
  useSharedPoll(queryKey, query.data?.supported === false ? false : interval)
  return query
}
