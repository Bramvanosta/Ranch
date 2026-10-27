import { useNavigate } from "@tanstack/react-router"
import { useEffect } from "react"

import { useCommandHandler } from "@/commands/handlers"
import { useRegistryShortcuts } from "@/commands/useRegistryShortcuts"
import { useSetTheme } from "@/lib/settingsSync"
import { SETTINGS_TABS, type SettingsTab } from "@/lib/windows/settingsWindow"

/**
 * What the menu bar does while the Settings window has the
 * focus: Settings… stays here, the theme commands apply everywhere. The
 * commands about connections and queues wait for a Ranch window (greyed).
 */
export function SettingsWindowCommands() {
  useRegistryShortcuts()
  const setTheme = useSetTheme()
  useCommandHandler("app.settings", () => {})
  useCommandHandler("view.theme.light", () => setTheme("light"))
  useCommandHandler("view.theme.dark", () => setTheme("dark"))
  useCommandHandler("view.theme.system", () => setTheme("system"))
  return null
}

/** ⌘, from another window on a given page (`settings:show-tab`): show it. */
export function useSettingsWindowTab() {
  const navigate = useNavigate()
  useEffect(() => {
    let off: (() => void) | undefined
    let cancelled = false
    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<string>("settings:show-tab", (e) => {
          if (!SETTINGS_TABS.includes(e.payload as SettingsTab)) return
          void navigate({ to: "/settings", search: { tab: e.payload as SettingsTab } })
        }),
      )
      .then((unlisten) => {
        if (cancelled) unlisten()
        else off = unlisten
      })
      .catch(() => {})
    return () => {
      cancelled = true
      off?.()
    }
  }, [navigate])
}
