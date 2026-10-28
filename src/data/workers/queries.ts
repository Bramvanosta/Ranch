import { useQuery } from "@tanstack/react-query"
import { unwrap } from "@/data/result"
import { useRefetchInterval } from "@/data/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { commands, type QueueId } from "@/lib/tauri/bindings"
import { workerKeys } from "./keys"

/**
 * The BullMQ workers attached to a queue.
 *
 * `list_workers` reads `CLIENT LIST`; a Redis that forbids it answers
 * `supported: false` (not an error: the actor probes once per connection).
 * Workers come and go and `idleSecs` moves every second, so the list
 * refreshes on its own every `WORKERS_REFRESH_MS`; the tab also has a manual
 * refresh. Once unsupported, polling stops: the answer cannot change until
 * the connection is re-opened.
 */
export const WORKERS_REFRESH_MS = 5_000

export interface UseWorkersArgs {
  connectionId: string
  queueId: QueueId
  /** `false` when the connection's backend lists no workers. */
  enabled?: boolean
}

export function useWorkers(args: UseWorkersArgs) {
  // Follows Settings › Refresh, never faster than 5 s.
  const interval = useRefetchInterval(WORKERS_REFRESH_MS)
  const queryKey = workerKeys.list(args.connectionId, args.queueId)
  const query = useQuery({
    queryKey,
    queryFn: () => fetchWorkers(args.connectionId, args.queueId),
    staleTime: WORKERS_REFRESH_MS,
    refetchInterval: false,
    retry: 1,
    enabled: args.enabled ?? true,
  })
  // One self-paced poller per query (see `useSharedPoll`).
  const polled = (args.enabled ?? true) && query.data?.supported !== false
  useSharedPoll(queryKey, polled ? interval : false)
  return query
}

/** `list_workers` for `workerKeys.list`, for a query that reads every queue of a connection. */
export async function fetchWorkers(connectionId: string, queueId: QueueId) {
  return unwrap(await commands.listWorkers({ connectionId, queueId }))
}
