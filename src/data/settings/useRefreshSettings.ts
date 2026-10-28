import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useCallback } from "react"
import { announceSettingsChange } from "@/lib/settingsSync"

import {
  DEFAULT_REFRESH_SETTINGS,
  effectiveInterval,
  type RefreshSettings,
  refreshAuto,
  refreshIntervalMs,
} from "./refreshSettings"
import { readSettingOrDefault, writeSetting } from "./useSetting"

const QUERY_KEY = ["kv", "refresh"] as const

/**
 * The auto-refresh setting, stored in `kv_settings`.
 * Until it has loaded, and if it cannot be read, polling behaves as it
 * always did (`DEFAULT_REFRESH_SETTINGS`): a settings hiccup must never
 * freeze the counters.
 */
export function useRefreshSettings() {
  const queryClient = useQueryClient()
  const { data } = useQuery({
    queryKey: QUERY_KEY,
    queryFn: async (): Promise<RefreshSettings> => {
      const [auto, intervalMs] = await Promise.all([
        readSettingOrDefault(refreshAuto),
        readSettingOrDefault(refreshIntervalMs),
      ])
      return { auto, intervalMs }
    },
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  })

  const mutation = useMutation({
    mutationFn: async (next: RefreshSettings) => {
      await Promise.all([
        writeSetting(refreshAuto, next.auto),
        writeSetting(refreshIntervalMs, next.intervalMs),
      ])
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
