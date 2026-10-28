import { type QueryClient, useQueryClient } from "@tanstack/react-query"
import { useTheme } from "next-themes"
import { useCallback, useEffect } from "react"

import { connectionKeys } from "@/data/connections/keys"
import { hasNativeWindows } from "@/lib/runtime/runtime"

/**
 * A setting changed in one window shows in every window at once
 * (the Settings window has its own). Each window keeps its own query
 * cache, so a change is announced: the desktop app broadcasts a Tauri event
 * to every window, a browser a `BroadcastChannel` message to its other tabs.
 *
 *   - `theme`: the other windows switch to it (`next-themes` is per window);
 *   - `kv`:    a `kv_settings` value changed: refresh interval, alert
 *              thresholds, page size, mutes, hints… (query keys `["kv", …]`);
 *   - `mcp`:   what agents may do (Settings › Agents › Safety).
 */
export type SettingsChange = { topic: "theme"; theme: string } | { topic: "kv" } | { topic: "mcp" }

const EVENT = "settings:changed"

let channel: BroadcastChannel | null = null
function browserChannel(): BroadcastChannel | null {
  if (typeof BroadcastChannel === "undefined") return null
  channel ??= new BroadcastChannel("ranch-settings")
  return channel
}

/** Tell the other windows. Call it once the change is saved. */
export function announceSettingsChange(change: SettingsChange): void {
  if (hasNativeWindows()) {
    void import("@tauri-apps/api/event").then(({ emit }) => emit(EVENT, change)).catch(() => {})
    return
  }
  browserChannel()?.postMessage(change)
}

/** What a window does with a change announced by another one (or itself). */
export function applySettingsChange(
  change: SettingsChange,
  queryClient: QueryClient,
  setTheme: (theme: string) => void,
): void {
  switch (change.topic) {
    case "theme":
      setTheme(change.theme)
      return
    case "kv":
      void queryClient.invalidateQueries({ queryKey: ["kv"] })
      return
    case "mcp":
      void queryClient.invalidateQueries({ queryKey: ["mcpSafetySettings"] })
      void queryClient.invalidateQueries({ queryKey: connectionKeys.all })
      return
  }
}

/** Mounted once per window, at the root: listens for changes. */
export function useSettingsSync(): void {
  const queryClient = useQueryClient()
  const { setTheme } = useTheme()
  useEffect(() => {
    const apply = (change: SettingsChange) => applySettingsChange(change, queryClient, setTheme)
    if (hasNativeWindows()) {
      let off: (() => void) | undefined
      let cancelled = false
      void import("@tauri-apps/api/event")
        .then(({ listen }) => listen<SettingsChange>(EVENT, (e) => apply(e.payload)))
        .then((unlisten) => {
          if (cancelled) unlisten()
          else off = unlisten
        })
        .catch(() => {})
      return () => {
        cancelled = true
        off?.()
      }
    }
    const ch = browserChannel()
    const onMessage = (e: MessageEvent<SettingsChange>) => apply(e.data)
    ch?.addEventListener("message", onMessage)
    return () => ch?.removeEventListener("message", onMessage)
  }, [queryClient, setTheme])
}

/** Set the theme here and in every other window. */
export function useSetTheme(): (theme: string) => void {
  const { setTheme } = useTheme()
  return useCallback(
    (theme: string) => {
      setTheme(theme)
      announceSettingsChange({ topic: "theme", theme })
    },
    [setTheme],
  )
}
