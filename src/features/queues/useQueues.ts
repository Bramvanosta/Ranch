import { useQuery } from "@tanstack/react-query"
import { LIVE_SAFETY_NET_MS, useIsLive } from "@/features/live/useConnectionEvents"
import { useRefetchInterval } from "@/features/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { commands } from "@/lib/tauri/bindings"

/**
 * TanStack Query hook for the queue list.
 *
 * Polls `list_queues` every 1 second while the document is visible.
 * `refetchIntervalInBackground: false` matches the reachability
 * indicator pattern — when the user switches to another
 * tab/desktop, polling pauses; on focus return, it resumes. Saves Redis
 * ops on idle windows (the budget is "while the user is looking").
 *
 * "every 1 second" is now the default of a user setting
 * (Settings → Refresh): another interval, or no polling at all.
 *
 * Schema-not-recognized failures don't retry — surface immediately so
 * the user sees the typed message. Transient failures retry once (Story
 * 18.5): the next poll is the next try, and the error shows sooner.
 */
/** The cheapest poll there is (one pipeline per queue): may run every second. */
export const QUEUES_REFRESH_FLOOR_MS = 1_000

/** `list_queues` for `["queues", connectionId]`, shared by every reader of the key. */
export async function fetchQueues(connectionId: string) {
  const result = await commands.listQueues({ connectionId })
  if (result.status === "error") {
    // Encode the typed kind in the error so the retry predicate can
    // distinguish schemaNotRecognized from transient failures.
    const err = new Error(result.error.message)
    ;(err as Error & { kind?: string }).kind = result.error.kind
    throw err
  }
  return result.data
}

/** A connection not on screen (the sidebar): at most this often. */
export const BACKGROUND_QUEUES_FLOOR_MS = 10_000

export function useQueues(connectionId: string, opts: { background?: boolean } = {}) {
  const setting = useRefetchInterval(QUEUES_REFRESH_FLOOR_MS)
  // Followed live, the counters are read when a queue moves:
  // the poll is a safety net.
  const live = useIsLive(connectionId)
  const interval = setting === false || !live ? setting : Math.max(setting, LIVE_SAFETY_NET_MS)
  // One self-paced poller for the key, as often as its most
  // eager reader asks: the open connection's screen every second, the
  // sidebar alone every 10 s.
  useSharedPoll(
    ["queues", connectionId],
    interval === false || !opts.background
      ? interval
      : Math.max(interval, BACKGROUND_QUEUES_FLOOR_MS),
  )
  return useQuery({
    queryKey: ["queues", connectionId],
    queryFn: () => fetchQueues(connectionId),
    refetchInterval: false,
    retry: (failureCount, error) => {
      const kind = (error as Error & { kind?: string }).kind
      // Don't retry on schemaNotRecognized — surface immediately.
      if (kind === "schemaNotRecognized") return false
      // Once: the next poll is the next try.
      return failureCount < 1
    },
  })
}
