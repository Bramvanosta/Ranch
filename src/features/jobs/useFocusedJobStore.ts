/**
 * module-level store for the keyboard-focused job-list
 * row index. Read by `JobListRow` to apply the highlight ring;
 * written by the `JobListKeyboardNavigation` mount in `JobList`.
 */
import { create } from "zustand"

interface FocusedJobState {
  /** `null` until the keyboard is used: no row is marked for a mouse user. */
  focusedIndex: number | null
  setFocusedIndex: (i: number | null) => void
}

export const useFocusedJobStore = create<FocusedJobState>((set) => ({
  focusedIndex: null,
  setFocusedIndex: (focusedIndex) => set({ focusedIndex }),
}))
