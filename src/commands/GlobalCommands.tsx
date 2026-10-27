import { useQueryClient } from "@tanstack/react-query"
import { useNavigate, useRouter } from "@tanstack/react-router"
import { useState } from "react"

import { useAiEnabled } from "@/lib/ai/aiFeatures"
import { useSetTheme } from "@/lib/settingsSync"
import { openRouteOrSay } from "@/lib/windows/openRoute"
import { useOpenSettings } from "@/lib/windows/settingsWindow"
import { useApprovalUiStore } from "@/stores/useApprovalUiStore"
import { useShellStore } from "@/stores/useShellStore"
import { useSidebarUiStore } from "@/stores/useSidebarUiStore"

import { useCommandHandler } from "./handlers"
import { ShortcutsDialog } from "./ShortcutsDialog"
import { useRegistryShortcuts } from "./useRegistryShortcuts"

/**
 * The commands that apply everywhere, whatever screen is open:
 * settings, new connection or group, refresh, appearance, sidebar, audit log,
 * keyboard shortcuts, agents. Mounted once at the root, with the window's
 * chord listener.
 */
export function GlobalCommands() {
  const aiOn = useAiEnabled()
  useRegistryShortcuts()
  const navigate = useNavigate()
  const router = useRouter()
  const queryClient = useQueryClient()
  const setTheme = useSetTheme()
  const openAddConnection = useSidebarUiStore((s) => s.openAddConnection)
  const requestNewGroup = useSidebarUiStore((s) => s.requestNewGroup)
  const sidebarHidden = useShellStore((s) => s.sidebarHidden)
  const toggleSidebar = useShellStore((s) => s.toggleSidebar)
  const showApprovals = useApprovalUiStore((s) => s.showNow)
  const [shortcutsOpen, setShortcutsOpen] = useState(false)

  const openSettings = useOpenSettings()
  // A window of its own on the desktop (focused if open).
  useCommandHandler("app.settings", () => openSettings())
  // This screen again, in a new window or a new tab.
  const here = () => router.state.location.href
  useCommandHandler("file.newWindow", () => openRouteOrSay(here(), { tab: false }))
  useCommandHandler("file.newTab", () => openRouteOrSay(here(), { tab: true }))
  useCommandHandler("file.newConnection", () => openAddConnection())
  useCommandHandler("file.newGroup", requestNewGroup)
  useCommandHandler("view.toggleSidebar", toggleSidebar, {
    label: sidebarHidden ? "Show Sidebar" : "Hide Sidebar",
  })
  // ⌘R refreshes what is on screen: every query that is being watched.
  useCommandHandler("view.refresh", () => void queryClient.invalidateQueries())
  useCommandHandler("view.theme.light", () => setTheme("light"))
  useCommandHandler("view.theme.dark", () => setTheme("dark"))
  useCommandHandler("view.theme.system", () => setTheme("system"))
  useCommandHandler("window.auditLog", () => void navigate({ to: "/audit" }))
  useCommandHandler("help.shortcuts", () => setShortcutsOpen(true))
  useCommandHandler("agents.pending", showApprovals, { enabled: aiOn })
  // The Agents page.
  const agentsTab = (tab: "activity" | "notes" | "setup") =>
    void navigate({ to: "/agents", search: { tab } })
  useCommandHandler("agents.activity", () => agentsTab("activity"), { enabled: aiOn })
  useCommandHandler("agents.notes", () => agentsTab("notes"), { enabled: aiOn })
  useCommandHandler("agents.setup", () => agentsTab("setup"), { enabled: aiOn })

  return <ShortcutsDialog open={shortcutsOpen} onOpenChange={setShortcutsOpen} />
}
