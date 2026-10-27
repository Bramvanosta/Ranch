import { create } from "zustand"

/** window-level view state (View › Show / Hide Sidebar). */
interface ShellState {
  sidebarHidden: boolean
  toggleSidebar: () => void
  /** View › Hide Inspector (⌥⌘I) keeps the opened job, out of sight. */
  inspectorHidden: boolean
  toggleInspector: () => void
}

export const useShellStore = create<ShellState>((set) => ({
  sidebarHidden: false,
  toggleSidebar: () => set((s) => ({ sidebarHidden: !s.sidebarHidden })),
  inspectorHidden: false,
  toggleInspector: () => set((s) => ({ inspectorHidden: !s.inspectorHidden })),
}))
