import { createRootRoute, Outlet } from "@tanstack/react-router"

import { GlobalCommands } from "@/commands/GlobalCommands"
import { NativeMenuBar } from "@/commands/NativeMenuBar"
import { useDeepLinkRouter } from "@/features/deep-link/useDeepLinkRouter"
import { McpApprovalGate } from "@/features/mcp/approval/McpApprovalGate"
import { useThresholdWatcher } from "@/features/notifications/useThresholdWatcher"
import { FirstRunWizard, useFirstRunWizardVisibility } from "@/features/onboarding/FirstRunWizard"
import { CommandPalette } from "@/features/palette/CommandPalette"
import {
  SettingsWindowCommands,
  useSettingsWindowTab,
} from "@/features/settings/SettingsWindowCommands"
import { usePersistSelection } from "@/hooks/usePersistSelection"
import { AppShell } from "@/layouts/AppShell"
import { useAiEnabled, useAiEnabledSync } from "@/lib/ai/aiFeatures"
import { useSettingsSync } from "@/lib/settingsSync"
import { isSettingsWindow } from "@/lib/windows/settingsWindow"

/**
 * Root route — owns the AppShell (sidebar + screen). Every concrete route
 * renders its content into the screen area via `<Outlet/>`. The
 * the sidebar (ConnectionSidebar) live inside AppShell itself.
 *
 * The selection-persist hook mounts here so it's active for the entire
 * router lifetime, regardless of which child route is rendered.
 *
 * `CommandPalette` mounts at the root so its global
 * cmd-K binding is active everywhere in the app.
 */
export const Route = createRootRoute({
  component: RootComponent,
})

function RootComponent() {
  // The Settings window is Settings only: no sidebar, palette,
  // approvals, watchers or welcome screen (the main window has them).
  return isSettingsWindow() ? <SettingsWindowRoot /> : <MainRoot />
}

function SettingsWindowRoot() {
  useSettingsSync()
  useAiEnabledSync()
  useSettingsWindowTab()
  return (
    <div className="h-screen bg-(--bg) text-(--text)">
      <Outlet />
      <NativeMenuBar />
      <SettingsWindowCommands />
    </div>
  )
}

function MainRoot() {
  // Settings changed in another window apply here at once.
  useSettingsSync()
  // AI features on or off.
  useAiEnabledSync()
  const aiOn = useAiEnabled()
  usePersistSelection()
  // Listen for `ranch://` deep-link events and route.
  useDeepLinkRouter()
  // Watch queue counters and fire native notifications
  // on configured threshold crossings.
  useThresholdWatcher()
  // first-run wizard visibility (decides post-mount).
  const { show, resolved, setShow } = useFirstRunWizardVisibility()
  return (
    <>
      <AppShell>
        <Outlet />
        {/* The registry commands that apply everywhere, and the chords. */}
        <GlobalCommands />
        <CommandPalette />
        {/* Agent write approvals; the one modal allowed to open unprompted. */}
        {/* Not in the server image: no MCP there, nothing to approve or poll. */}
        {/* Nor with AI features off: the requests were denied. */}
        {aiOn ? <McpApprovalGate /> : null}
      </AppShell>
      {resolved && show ? <FirstRunWizard onClose={() => setShow(false)} /> : null}
    </>
  )
}
