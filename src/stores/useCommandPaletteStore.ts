import { create } from "zustand"

/**
 * cmd-K command palette open state + recent-items list.
 *
 * The palette is a singleton overlay; only one can be open at a time.
 * The store holds:
 *   - `open`: whether the palette is rendered.
 *   - `recents`: most-recently-visited entries (capped at 10).
 *
 * Recents persist via `kv_settings` (key `palette.recents`); the
 * write-through is done by the route effect, not
 * the store itself, so the store stays pure.
 */
export type RecentKind = "connection" | "queue" | "audit" | "settings"

export interface RecentItem {
  kind: RecentKind
  id: string
  label: string
  /** Optional secondary label (e.g. connection name for a queue). */
  hint?: string
  /** Route path to navigate to on Enter. */
  to: string
  /** ISO timestamp for ordering + trim. */
  ts: string
}

const MAX_RECENTS = 10

interface CommandPaletteState {
  open: boolean
  /** Text the palette opens with (the sidebar filter hands it over). */
  initialQuery: string
  recents: RecentItem[]
  setOpen: (open: boolean) => void
  /** Open the palette with `query` already typed. */
  openWith: (query: string) => void
  toggle: () => void
  /** Push (or refresh) a recent item. Dedupes by `kind + id`. */
  pushRecent: (item: Omit<RecentItem, "ts">) => void
  /** Hydrate from `kv_settings` payload at app start. */
  hydrateRecents: (items: RecentItem[]) => void
}

export const useCommandPaletteStore = create<CommandPaletteState>((set) => ({
  open: false,
  initialQuery: "",
  recents: [],
  setOpen: (open) => set(open ? { open } : { open, initialQuery: "" }),
  openWith: (query) => set({ open: true, initialQuery: query }),
  toggle: () => set((s) => ({ open: !s.open })),
  pushRecent: (item) =>
    set((s) => {
      const now = new Date().toISOString()
      const existing = s.recents.filter((r) => !(r.kind === item.kind && r.id === item.id))
      const next: RecentItem[] = [{ ...item, ts: now }, ...existing].slice(0, MAX_RECENTS)
      return { recents: next }
    }),
  hydrateRecents: (items) => set({ recents: items.slice(0, MAX_RECENTS) }),
}))
