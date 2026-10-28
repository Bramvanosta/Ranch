import { create } from "zustand"

/**
 * UI state shared by `McpApprovalGate` (root) and the status bar.
 *
 * `deferred` = a request is waiting but the modal is held back because another
 * modal is open (modals never stack). The status-bar item then is the way in:
 * `showNow()` lifts the hold.
 */
interface ApprovalUiState {
  deferred: boolean
  setDeferred: (deferred: boolean) => void
  /** Bumped by the status-bar item; the gate shows the modal regardless of other dialogs. */
  showRequest: number
  showNow: () => void
}

export const useApprovalUiStore = create<ApprovalUiState>((set) => ({
  deferred: false,
  setDeferred: (deferred) => set({ deferred }),
  showRequest: 0,
  showNow: () => set((s) => ({ showRequest: s.showRequest + 1, deferred: false })),
}))
