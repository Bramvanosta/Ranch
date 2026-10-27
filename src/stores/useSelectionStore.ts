import { create } from "zustand"

/**
 * `useSelectionStore` — first store in the repo.
 *
 * Tracks the currently-selected connection. The URL is the canonical source
 * of truth (via TanStack Router's `useParams`), but mirroring it into a
 * Zustand store lets non-route components (status bar, future breadcrumb,
 * future title bar) read the selection without needing router context.
 *
 * Pattern other Ranch stores follow:
 *   - One store per concern.
 *   - Plain `create<State>()` — no middleware needed for in-memory state.
 *   - Cross-restart persistence is NOT done in the store itself; a separate
 *     write-through hook (`usePersistSelection`) handles `kv_settings` IO.
 *     This keeps the store pure and avoids coupling Zustand to the Tauri
 *     bridge.
 */
interface SelectionState {
  selectedConnectionId: string | null
  select: (id: string | null) => void
}

export const useSelectionStore = create<SelectionState>((set) => ({
  selectedConnectionId: null,
  select: (id) => set({ selectedConnectionId: id }),
}))
