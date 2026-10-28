import { useQuery } from "@tanstack/react-query"
import { unwrap } from "@/data/result"
import { useRefetchInterval } from "@/data/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { commands } from "@/lib/tauri/bindings"
import { serverKeys } from "./keys"

/** INFO is cheap but not free: every 10 s is plenty for an info strip. */
export const SERVER_INFO_FLOOR_MS = 10_000

/** `get_server_info` for `serverKeys.info(connectionId)`, shared by every reader of the key. */
export async function fetchServerInfo(connectionId: string) {
  return unwrap(await commands.getServerInfo({ connectionId }))
}

/** The server's version, latency, memory and load, polled while `enabled`. */
export function useServerInfo(connectionId: string, enabled: boolean) {
  const refetchInterval = useRefetchInterval(SERVER_INFO_FLOOR_MS)
  // One self-paced poller per query, shared (see `useSharedPoll`).
  const queryKey = serverKeys.info(connectionId)
  useSharedPoll(queryKey, enabled ? refetchInterval : false)
  return useQuery({
    queryKey,
    queryFn: () => fetchServerInfo(connectionId),
    enabled,
    refetchInterval: false,
    retry: false,
  })
}
