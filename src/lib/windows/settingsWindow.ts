import { useNavigate } from "@tanstack/react-router"
import { useCallback } from "react"
import { toast } from "sonner"

import { getRuntime } from "@/lib/runtime/runtime"
import { commands } from "@/lib/tauri/bindings"

import { currentWindowLabel } from "./openRoute"

/** The pages of Settings, in toolbar order. */
export const SETTINGS_TABS = [
  "general",
  "appearance",
  "refresh",
  "alerts",
  "updates",
  "advanced",
  "ai",
] as const
export type SettingsTab = (typeof SETTINGS_TABS)[number]

/** This window is the Settings window (desktop): no sidebar, no palette. */
export function isSettingsWindow(): boolean {
  return getRuntime() === "desktop" && currentWindowLabel() === "settings"
}

/**
 * Ranch › Settings… (⌘,): the desktop app opens its one Settings window, or
 * brings it to the front on the asked page; a browser shows the
 * Settings page in place, as it has no windows to open.
 */
export function useOpenSettings(): (tab?: SettingsTab) => void {
  const navigate = useNavigate()
  return useCallback(
    (tab?: SettingsTab) => {
      if (getRuntime() === "desktop") {
        void commands.openSettingsWindow({ tab: tab ?? null }).then((r) => {
          if (r.status === "error") toast.error(r.error.message)
        })
        return
      }
      void navigate({ to: "/settings", search: { tab: tab ?? "general" } })
    },
    [navigate],
  )
}
