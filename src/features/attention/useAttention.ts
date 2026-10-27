import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useMemo } from "react"

import { useRefetchInterval } from "@/features/settings/useRefreshSettings"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { announceSettingsChange } from "@/lib/settingsSync"
import { type AttentionItem, commands } from "@/lib/tauri/bindings"

/** The report reads every connection: never more often than this. */
const ATTENTION_FLOOR_MS = 30_000
export const ATTENTION_QUERY_KEY = ["attention"] as const
const MUTES_KEY = "attention_mutes"

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
      const r = await commands.listAttention({ pausedThresholdMs: null })
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    refetchInterval: false,
    staleTime: 10_000,
  })
}

/** Muted items: key → until (ms), kept in the kv store (`attention_mutes`). */
export function useAttentionMutes() {
  const qc = useQueryClient()
  const query = useQuery({
    queryKey: ["kv", MUTES_KEY],
    queryFn: async () => {
      const r = await commands.getKvSetting(MUTES_KEY)
      if (r.status === "error" || !r.data) return {} as Record<string, number>
      try {
        return JSON.parse(r.data) as Record<string, number>
      } catch {
        return {}
      }
    },
  })
  const save = useMutation({
    mutationFn: async (next: Record<string, number>) => {
      const r = await commands.setKvSetting(MUTES_KEY, JSON.stringify(next))
      if (r.status === "error") throw new Error(r.error.message)
      return next
    },
    onSuccess: (next) => {
      qc.setQueryData(["kv", MUTES_KEY], next)
      announceSettingsChange({ topic: "kv" })
    },
  })
  const now = Date.now()
  // Expired mutes are simply ignored (and dropped at the next save).
  const active = useMemo(
    () => Object.fromEntries(Object.entries(query.data ?? {}).filter(([, until]) => until > now)),
    [query.data, now],
  )
  return {
    mutes: active,
    mute: (key: string, until: number) => save.mutate({ ...active, [key]: until }),
    unmute: (key: string) => {
      const { [key]: _, ...rest } = active
      save.mutate(rest)
    },
    unmuteAll: () => save.mutate({}),
  }
}

export function isMuted(item: Pick<AttentionItem, "key">, mutes: Record<string, number>): boolean {
  return (mutes[item.key] ?? 0) > Date.now()
}
