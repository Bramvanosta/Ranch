import { Link } from "@tanstack/react-router"
import {
  LayoutGridIcon,
  PlusIcon,
  ScrollTextIcon,
  SettingsIcon,
  SparklesIcon,
  TriangleAlertIcon,
} from "lucide-react"
import type { MouseEvent, ReactNode } from "react"
import { displayChord } from "@/commands/chords"
import { runCommand } from "@/commands/handlers"
import { NativeMenuBar } from "@/commands/NativeMenuBar"
import { RanchMenuButton } from "@/commands/RanchMenuButton"
import { RanchMark } from "@/components/RanchMark"
import { Button } from "@/components/ui/button"
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable"
import { ConnectionSidebar } from "@/features/connections/ConnectionSidebar"
import { SidebarFilterField } from "@/features/connections/SidebarFilterView"
import { handleSidebarKeyDown } from "@/features/connections/sidebarKeyboard"
import { useAttentionCount } from "@/features/connections/useAttentionCount"
import { usePendingApprovals } from "@/features/mcp/approval/usePendingApprovals"
import { UpdaterStatusBadge } from "@/features/updater/UpdaterStatusBadge"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import { getRuntime, getServerInfo, hasOverlayTitleBar, hasUpdater } from "@/lib/runtime/runtime"
import { cn } from "@/lib/utils"
import { useOpenSettings } from "@/lib/windows/settingsWindow"
import { useShellStore } from "@/stores/useShellStore"

/**
 * The window: one resizable
 * sidebar (220–320 px, View › Hide Sidebar) and the screen. Each route lays
 * itself out with `<Screen>` (toolbar, info strip, view tabs, content, inspector).
 *
 * The desktop app has a native menu bar (`NativeMenuBar`); the browser
 * runtimes get the same registry from a "Ranch" button in the sidebar header.
 * On macOS the window has no title bar (`titleBarStyle: Overlay`): the sidebar
 * keeps the toolbar's height for the traffic lights and drags the window.
 * There is no footer: the version is in Ranch › About, an update shows at the
 * bottom of the sidebar, pending approvals on the Agents item.
 */
/** The mark and the name, not a control: lets clicks through to the drag region. */
function AppName() {
  return (
    <span
      className="pointer-events-none flex items-center gap-1.5 text-sm font-semibold text-(--text) select-none"
      data-testid="app-name"
    >
      <RanchMark size={18} />
      <span className="font-mono">ranch</span>
    </span>
  )
}

export function AppShell({ children }: { children?: ReactNode }) {
  const overlay = hasOverlayTitleBar()
  const desktop = getRuntime() === "desktop"
  const sidebarHidden = useShellStore((s) => s.sidebarHidden)
  return (
    // `w-full`, not `w-screen`: 100vw counts a scrollbar gutter the content cannot use.
    <div className="flex h-full w-full flex-col bg-(--bg) text-(--text)">
      {desktop ? <NativeMenuBar /> : null}
      <ResizablePanelGroup orientation="horizontal" className="min-h-0 flex-1">
        {sidebarHidden ? null : (
          <>
            <ResizablePanel
              id="sidebar"
              defaultSize="240px"
              minSize="220px"
              maxSize="320px"
              className="bg-(--side)"
            >
              {/* ↑↓←→ Enter across the whole tree. */}
              <aside
                className="flex h-full flex-col"
                aria-label="Sidebar"
                onKeyDown={handleSidebarKeyDown}
              >
                {/* The desktop app's menus are in the menu bar, so its sidebar
                    header only carries the name — beside the traffic lights on
                    macOS (the row drags the window), on its own row elsewhere. */}
                {overlay ? (
                  <div
                    data-tauri-drag-region
                    data-testid="traffic-light-spacer"
                    className="flex h-toolbar shrink-0 items-center justify-end px-3"
                  >
                    <AppName />
                  </div>
                ) : desktop ? (
                  <div className="flex shrink-0 items-center px-3 pt-3">
                    <AppName />
                  </div>
                ) : null}
                {desktop ? null : (
                  <div className="flex shrink-0 items-center px-2 pt-2">
                    <RanchMenuButton />
                  </div>
                )}
                <div className="flex min-h-0 flex-1 flex-col">
                  <SidebarFilterField />
                  <SidebarNav />
                  <div className="min-h-0 flex-1">
                    <ConnectionSidebar />
                  </div>
                </div>
                <SidebarFooter />
              </aside>
            </ResizablePanel>
            <ResizableHandle />
          </>
        )}
        <ResizablePanel id="content" minSize="600px">
          {/* The screen scrolls inside its views, never the window as a whole. */}
          <main
            className="flex h-full min-w-0 flex-col overflow-hidden text-sm"
            aria-label="Content"
          >
            <div className="min-h-0 flex-1">{children}</div>
          </main>
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  )
}

/**
 * The app's own places, above the connections. Needs attention
 * opens the cross-connection view; Agents points at the closest
 * screen that exists today until the Agents page.
 */
function SidebarNav() {
  const aiOn = useAiEnabled()
  const openSettings = useOpenSettings()
  const attention = useAttentionCount()
  return (
    <nav aria-label="Main" className="flex shrink-0 flex-col gap-px px-2 pt-2 pb-1">
      <NavLink to="/" exact icon={<LayoutGridIcon />} testId="nav-home">
        All connections
      </NavLink>
      <NavLink
        to="/attention"
        icon={<TriangleAlertIcon />}
        testId="nav-attention"
        count={attention > 0 ? attention : undefined}
        tone="danger"
      >
        Needs attention
      </NavLink>
      {aiOn ? <AgentsLink /> : null}
      <NavLink to="/audit" icon={<ScrollTextIcon />} testId="nav-audit">
        Audit log
      </NavLink>
      <NavLink
        to="/settings"
        icon={<SettingsIcon />}
        testId="nav-settings"
        onOpen={(e) => {
          // The Settings window, on the desktop.
          if (getRuntime() !== "desktop") return
          e.preventDefault()
          openSettings()
        }}
      >
        Settings
      </NavLink>
    </nav>
  )
}

function AgentsLink() {
  const pending = usePendingApprovals()
  return (
    <NavLink
      to="/agents"
      icon={<SparklesIcon />}
      testId="nav-agents"
      count={pending.length > 0 ? pending.length : undefined}
      tone="ai"
    >
      Agents
    </NavLink>
  )
}

function NavLink({
  to,
  search,
  exact = false,
  inactive = false,
  icon,
  testId,
  count,
  tone,
  onOpen,
  children,
}: {
  to: "/" | "/attention" | "/audit" | "/settings" | "/agents"
  /** Runs on click; `preventDefault` keeps the route. */
  onOpen?: (e: MouseEvent) => void
  search?: { tab: "ai" }
  exact?: boolean
  /** Never shown as the current page (its own screen does not exist yet). */
  inactive?: boolean
  icon: ReactNode
  testId: string
  count?: number
  tone?: "danger" | "ai"
  children: ReactNode
}) {
  return (
    <Link
      to={to}
      search={search}
      activeOptions={{ exact, includeSearch: false }}
      data-sidebar-row
      data-testid={testId}
      onClick={onOpen}
      className="flex min-h-row-queue items-center gap-2 rounded-control px-2 text-sm text-(--text2) outline-none hover:bg-(--surf2) hover:text-(--text) focus-visible:ring-2 focus-visible:ring-(--accent)/50 [&_svg]:size-4 [&_svg]:shrink-0"
      activeProps={inactive ? undefined : { className: "bg-(--sel) text-(--text) font-medium" }}
    >
      {icon}
      <span className="min-w-0 flex-1">{children}</span>
      {count !== undefined ? (
        <span
          className={cn(
            "font-tabular text-xs",
            tone === "danger" && "text-(--state-failed-text)",
            tone === "ai" && "text-(--ai)",
          )}
        >
          {count}
        </span>
      ) : null}
    </Link>
  )
}

/**
 * The bottom of the sidebar: New connection (⇧⌘N, a
 * registry command), the update notice when one is ready, and in the server
 * image who is signed in, the runtime and the version (no native About there).
 */
function SidebarFooter() {
  const server = getRuntime() === "server"
  return (
    <div className="flex shrink-0 flex-col gap-1 border-t border-(--line) p-2">
      {hasUpdater() ? <UpdaterStatusBadge /> : null}
      <Button
        variant="ghost"
        size="sm"
        className="w-full justify-start text-(--text2)"
        shortcut={displayChord("cmd+shift+n")}
        onClick={() => runCommand("file.newConnection")}
        data-testid="sidebar-new-connection"
      >
        <PlusIcon aria-hidden="true" />
        <span className="flex-1 text-left">New connection</span>
      </Button>
      {server ? (
        <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 px-2 text-2xs text-(--text2)">
          <SignedInAs />
          <span
            title="Ranch server: this page talks to ranch-server over HTTP. Update it by pulling a newer image."
            data-testid="runtime-server"
          >
            server
          </span>
          <span className="font-tabular" data-testid="app-version">
            v{__APP_VERSION__}
          </span>
        </div>
      ) : null}
    </div>
  )
}

/** Server image with OIDC: who is signed in, and the way out. */
function SignedInAs() {
  const info = getServerInfo()
  if (!info?.user || !info.logoutUrl) return null
  const who = info.user.name ?? info.user.email ?? "signed in"
  return (
    <span className="flex items-center gap-2 text-2xs text-(--text2)" data-testid="signed-in-as">
      <span title={info.user.email ?? undefined}>{who}</span>
      <a href={info.logoutUrl} className="hover:text-(--text) hover:underline">
        Sign out
      </a>
    </span>
  )
}
