/**
 * Settings: toolbar tabs General, Appearance, Refresh,
 * Alerts, Updates, Advanced and AI features (`?tab=`). On the desktop it lives in
 * its own window (`open_settings_window`, ⌘,); in a browser — the server
 * image, the preview — and if the main window lands here, it is a page.
 */
import { createFileRoute, useNavigate } from "@tanstack/react-router"
import { z } from "zod"

import { SettingsPage } from "@/features/settings/SettingsPage"
import { hasAgents } from "@/lib/runtime/runtime"
import { isSettingsWindow, SETTINGS_TABS, type SettingsTab } from "@/lib/windows/settingsWindow"

export const Route = createFileRoute("/settings")({
  component: SettingsRoute,
  validateSearch: z.object({
    // The old Notifications page is Alerts now; Agents is AI features (16.1).
    tab: z
      .preprocess(
        (v) => (v === "notifications" ? "alerts" : v === "agents" ? "ai" : v),
        z.enum(SETTINGS_TABS),
      )
      .default("general")
      .catch("general"),
  }),
})

function SettingsRoute() {
  const search = Route.useSearch()
  const navigate = useNavigate({ from: Route.fullPath })
  // The server image has no MCP: a link to Agents lands on General.
  const tab: SettingsTab = search.tab === "ai" && !hasAgents() ? "general" : search.tab
  return (
    <SettingsPage
      tab={tab}
      windowed={isSettingsWindow()}
      onTab={(next) => void navigate({ search: { tab: next }, replace: true })}
    />
  )
}
