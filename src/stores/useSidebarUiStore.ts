import { create } from "zustand"

/**
 * Sidebar intents raised from elsewhere (group / environment
 * context menus, the cmd-K palette). The sidebar owns the dialogs and inline
 * inputs; this store is how other components ask for them.
 *
 * Each intent is cleared by the sidebar once it has been honoured.
 */
interface SidebarUiState {
  /** Open the "Add connection" dialog, optionally preselecting a placement. */
  addConnectionTarget: { groupId?: string; environmentId?: string } | null
  openAddConnection: (target?: { groupId?: string; environmentId?: string }) => void
  closeAddConnection: () => void

  /** Show the inline "New group" input. */
  newGroupRequested: boolean
  requestNewGroup: () => void
  clearNewGroupRequest: () => void

  /** Expand + scroll to + focus this group's header ("Go to group"). */
  revealGroupId: string | null
  /** Start an inline rename on this group ("Rename group…"). */
  renameGroupId: string | null
  revealGroup: (id: string, opts?: { rename?: boolean }) => void
  clearReveal: () => void
}

export const useSidebarUiStore = create<SidebarUiState>((set) => ({
  addConnectionTarget: null,
  openAddConnection: (target) => set({ addConnectionTarget: target ?? {} }),
  closeAddConnection: () => set({ addConnectionTarget: null }),

  newGroupRequested: false,
  requestNewGroup: () => set({ newGroupRequested: true }),
  clearNewGroupRequest: () => set({ newGroupRequested: false }),

  revealGroupId: null,
  renameGroupId: null,
  revealGroup: (id, opts) => set({ revealGroupId: id, renameGroupId: opts?.rename ? id : null }),
  clearReveal: () => set({ revealGroupId: null, renameGroupId: null }),
}))
