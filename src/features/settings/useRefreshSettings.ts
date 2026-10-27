import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useCallback } from "react"

import { announceSettingsChange } from "@/lib/settingsSync"
import { commands } from "@/lib/tauri/bindings"

import {
  DEFAULT_REFRESH_SETTINGS,
  effectiveInterval,
  KV_REFRESH_AUTO,
  KV_REFRESH_INTERVAL,
  parseRefreshSettings,
  type RefreshSettings,
} from "./refreshSettings"

const QUERY_KEY = ["kv", "refresh"] as const

/**
 * The auto-refresh setting, stored in `kv_settings`.
 * Until it has loaded — and if it cannot be read — polling behaves as it
 * always did (`DEFAULT_REFRESH_SETTINGS`): a settings hiccup must never
 * freeze the counters.
 */
export function useRefreshSettings() {
  const queryClient = useQueryClient()
  const { data } = useQuery({
    queryKey: QUERY_KEY,
    queryFn: async (): Promise<RefreshSettings> => {
      try {
        const [auto, interval] = await Promise.all([
          commands.getKvSetting(KV_REFRESH_AUTO),
          commands.getKvSetting(KV_REFRESH_INTERVAL),
        ])
        return parseRefreshSettings(
          auto.status === "ok" ? auto.data : null,
          interval.status === "ok" ? interval.data : null,
        )
      } catch {
        return DEFAULT_REFRESH_SETTINGS
      }
    },
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  })

  const mutation = useMutation({
    mutationFn: async (next: RefreshSettings) => {
      const results = await Promise.all([
        commands.setKvSetting(KV_REFRESH_AUTO, next.auto ? "1" : "0"),
        commands.setKvSetting(KV_REFRESH_INTERVAL, String(next.intervalMs)),
      ])
      for (const r of results) {
        if (r.status === "error") throw new Error(r.error.message)
      }
    },
    // Every window polls at the new interval.
    onSuccess: () => announceSettingsChange({ topic: "kv" }),
  })
  const mutate = mutation.mutate
  const update = useCallback(
    (next: RefreshSettings) => {
      // Applied at once; persisted behind.
      queryClient.setQueryData(QUERY_KEY, next)
      mutate(next)
    },
    [queryClient, mutate],
  )

  return { settings: data ?? DEFAULT_REFRESH_SETTINGS, update }
}

/**
 * `refetchInterval` for a polled view: the user's interval, never below the
 * view's `floorMs`; `false` when auto-refresh is off.
 */
export function useRefetchInterval(floorMs: number): number | false {
  const { settings } = useRefreshSettings()
  return effectiveInterval(settings, floorMs)
}
