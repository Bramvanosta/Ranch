/**
 * The command registry: every user
 * action that has a place in the menu bar, the palette or a global shortcut.
 *
 * A definition says where the command sits and which chord runs it. What it
 * does, and whether it applies right now, comes from whoever can do it: the
 * component or route that owns the action registers a handler with
 * `useCommandHandler` (see `handlers.ts`). No handler, or a disabled one, means
 * the menu item is greyed out and the chord does nothing.
 */

export type MenuName =
  | "Ranch"
  | "File"
  | "Edit"
  | "View"
  | "Connection"
  | "Queue"
  | "Job"
  | "Agents"
  | "Window"
  | "Help"

export const MENU_ORDER: MenuName[] = [
  "Ranch",
  "File",
  "Edit",
  "View",
  "Connection",
  "Queue",
  "Job",
  "Agents",
  "Window",
  "Help",
]

export interface CommandDef {
  id: string
  /** Default label; a handler may replace it ("Resume “emails”"). */
  label: string
  menu: MenuName
  /** Chord in registry notation (`chords.ts`). */
  shortcut?: string
  /** A submenu inside `menu` (View › Appearance). */
  submenu?: string
  /** Items with a different section number are separated in the menu. */
  section?: number
  /** Agents menu: only when AI features are on. */
  agents?: boolean
}

export const COMMANDS: CommandDef[] = [
  // Ranch: About, Hide, Quit are the platform's own items (desktop).
  { id: "app.checkUpdates", label: "Check for Updates…", menu: "Ranch", section: 1 },
  { id: "app.settings", label: "Settings…", menu: "Ranch", shortcut: "cmd+comma", section: 2 },

  // Native windows and tabs (a tab is a window outside macOS).
  { id: "file.newWindow", label: "New Window", menu: "File", shortcut: "cmd+n" },
  { id: "file.newTab", label: "New Tab", menu: "File", shortcut: "cmd+t" },
  {
    id: "file.newConnection",
    label: "New Connection…",
    menu: "File",
    shortcut: "cmd+shift+n",
    section: 1,
  },
  { id: "file.newGroup", label: "New Group…", menu: "File", section: 1 },

  // Edit: Undo … Select All are the platform's own items (desktop).
  { id: "job.copyId", label: "Copy Job ID", menu: "Edit", shortcut: "cmd+shift+c", section: 1 },
  { id: "edit.find", label: "Find", menu: "Edit", shortcut: "cmd+f", section: 1 },

  { id: "view.toggleSidebar", label: "Hide Sidebar", menu: "View", shortcut: "ctrl+cmd+s" },
  { id: "view.toggleInspector", label: "Hide Inspector", menu: "View", shortcut: "alt+cmd+i" },
  { id: "view.state.waiting", label: "Waiting", menu: "View", shortcut: "cmd+1", section: 1 },
  { id: "view.state.active", label: "Active", menu: "View", shortcut: "cmd+2", section: 1 },
  { id: "view.state.delayed", label: "Delayed", menu: "View", shortcut: "cmd+3", section: 1 },
  { id: "view.state.failed", label: "Failed", menu: "View", shortcut: "cmd+4", section: 1 },
  { id: "view.state.completed", label: "Completed", menu: "View", shortcut: "cmd+5", section: 1 },
  {
    id: "view.groupByCause",
    label: "Group by Cause",
    menu: "View",
    shortcut: "alt+cmd+g",
    section: 1,
  },
  { id: "view.refresh", label: "Refresh", menu: "View", shortcut: "cmd+r", section: 2 },
  // The Map tab fits in the window, or shows at actual size.
  { id: "view.fitMap", label: "Fit Map", menu: "View", shortcut: "cmd+0" },
  { id: "view.theme.light", label: "Light", menu: "View", submenu: "Appearance", section: 2 },
  { id: "view.theme.dark", label: "Dark", menu: "View", submenu: "Appearance", section: 2 },
  { id: "view.theme.system", label: "System", menu: "View", submenu: "Appearance", section: 2 },

  { id: "connection.edit", label: "Edit Connection…", menu: "Connection", shortcut: "cmd+e" },
  {
    id: "connection.unlock",
    label: "Unlock for Session",
    menu: "Connection",
    shortcut: "cmd+shift+u",
    section: 1,
  },
  {
    id: "connection.lock",
    label: "Lock Now",
    menu: "Connection",
    shortcut: "cmd+shift+l",
    section: 1,
  },
  { id: "connection.copyUrl", label: "Copy Redis URL", menu: "Connection", section: 2 },
  { id: "connection.remove", label: "Remove Connection…", menu: "Connection", section: 3 },

  { id: "queue.pause", label: "Pause Queue", menu: "Queue", shortcut: "cmd+shift+p" },
  { id: "queue.addJob", label: "Add Job…", menu: "Queue", shortcut: "cmd+j" },
  {
    id: "queue.retryAllFailed",
    label: "Retry All Failed…",
    menu: "Queue",
    shortcut: "alt+cmd+r",
    section: 1,
  },
  { id: "queue.clean", label: "Clean…", menu: "Queue", section: 1 },
  { id: "queue.drain", label: "Drain…", menu: "Queue", section: 1 },
  {
    id: "queue.copyKey",
    label: "Copy Queue ID",
    menu: "Queue",
    shortcut: "alt+cmd+k",
    section: 2,
  },
  { id: "queue.openInNewTab", label: "Open in New Tab", menu: "Queue", section: 3 },

  { id: "job.retry", label: "Retry", menu: "Job", shortcut: "cmd+shift+r" },
  { id: "job.promote", label: "Promote", menu: "Job", shortcut: "alt+cmd+p" },
  { id: "job.remove", label: "Remove…", menu: "Job", shortcut: "cmd+backspace" },
  { id: "job.copyPayload", label: "Copy Payload", menu: "Job", shortcut: "alt+cmd+c", section: 1 },
  {
    id: "job.openInNewWindow",
    label: "Open in New Window",
    menu: "Job",
    shortcut: "alt+enter",
    section: 2,
  },

  {
    id: "agents.pending",
    label: "Pending Approvals…",
    menu: "Agents",
    shortcut: "alt+cmd+a",
    agents: true,
  },
  {
    id: "agents.askAi",
    label: "Ask AI About Selection",
    menu: "Agents",
    shortcut: "shift+alt+cmd+a",
    agents: true,
  },
  { id: "agents.activity", label: "Activity", menu: "Agents", agents: true },
  { id: "agents.notes", label: "Notes", menu: "Agents", agents: true },
  { id: "agents.setup", label: "Setup & Safety…", menu: "Agents", agents: true, section: 1 },

  { id: "window.auditLog", label: "Audit Log", menu: "Window", shortcut: "alt+cmd+l", section: 1 },

  { id: "help.shortcuts", label: "Keyboard Shortcuts", menu: "Help" },
]

export const commandById = new Map(COMMANDS.map((c) => [c.id, c]))
