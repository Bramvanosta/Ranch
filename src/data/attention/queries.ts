import { useQuery } from "@tanstack/react-query"
import { useMemo } from "react"
import { unwrap } from "@/data/result"
import { attentionMutes } from "@/data/settings/settings"
import { useRefetchInterval } from "@/data/settings/useRefreshSettings"
import { useSetting } from "@/data/settings/useSetting"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { type AttentionItem, commands } from "@/lib/tauri/bindings"

/** The report reads every connection: never more often than this. */
const ATTENTION_FLOOR_MS = 30_000
export const ATTENTION_QUERY_KEY = ["attention"] as const

/**
 * What needs the user across connections
 * (`list_attention`), most urgent first. Shared by the page and the sidebar
 * count.
 */
export function useAttentionReport() {
  const refetchInterval = useRefetchInterval(ATTENTION_FLOOR_MS)
  // One self-paced poller per query, shared (see `useSharedPoll`).
  const queryKey = ATTENTION_QUERY_KEY
  useSharedPoll(queryKey, refetchInterval)
  return useQuery({
    queryKey,
    queryFn: async () => {
      return unwrap(await commands.listAttention({ pausedThresholdMs: null }))
    },
    refetchInterval: false,
    staleTime: 10_000,
  })
}

/** Muted items: key → until (ms), kept in the kv store (`attention_mutes`). */
export function useAttentionMutes() {
  const { value: mutes, set: save } = useSetting(attentionMutes)
  const now = Date.now()
  // Expired mutes are simply ignored (and dropped at the next save).
  const active = useMemo(
    () => Object.fromEntries(Object.entries(mutes).filter(([, until]) => until > now)),
    [mutes, now],
  )
  return {
    mutes: active,
    mute: (key: string, until: number) => save({ ...active, [key]: until }),
    unmute: (key: string) => {
      const { [key]: _, ...rest } = active
      save(rest)
    },
    unmuteAll: () => save({}),
  }
}

export function isMuted(item: Pick<AttentionItem, "key">, mutes: Record<string, number>): boolean {
  return (mutes[item.key] ?? 0) > Date.now()
}
