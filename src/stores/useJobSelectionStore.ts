import { create } from "zustand"

import type { JobRef, JobState } from "@/lib/tauri/bindings"

/**
 * multi-select store for the JobList.
 *
 * Tracks selected jobs as a Map keyed by the job's queue id and id.
 * The Map shape lets:
 *   - O(1) presence checks (vs. Set<key> + side Map<key, entry>).
 *   - `distinctQueues()` computation in O(N) by iterating values
 *     and collecting their queue ids into a Set.
 *
 * The store is cleared when:
 *   - The user switches connection (subscribed in App.tsx via
 *     `useSelectionStore`).
 *   - The user switches queue or state filter (cleared in the
 *     route's `useEffect` since the route owns the URL state).
 *   - The user explicitly clicks Cancel in the `BulkActionBar`.
 *
 * Cross-queue selection (K > 1) is supported in the data layer
 * and by `BulkActionBar`.
 */
export interface JobSelectionEntry {
  jobRef: JobRef
  /** Snapshot of the job's state at selection time. Used by
   *  the BulkActionBar to enable/disable per-action affordances
   *  (e.g. Promote requires all selected to be `delayed`). */
  state: JobState
}

export const jobSelectionKey = (e: JobSelectionEntry): string =>
  `${e.jobRef.queueId}\u0000${e.jobRef.id}`

interface JobSelectionStore {
  selection: Map<string, JobSelectionEntry>
  toggle: (entry: JobSelectionEntry) => void
  add: (entry: JobSelectionEntry) => void
  addRange: (entries: JobSelectionEntry[]) => void
  remove: (key: string) => void
  removeRange: (keys: string[]) => void
  clear: () => void
  /**
   * A bulk action asked for from outside the `BulkActionBar`: the list's
   * `cmd-Backspace` on the focused row. The bar picks it up, opens its
   * confirmation step, and clears it.
   */
  requestedAction: "retry" | "remove" | "promote" | null
  requestAction: (action: "retry" | "remove" | "promote" | null) => void
  /** Replace the current selection with the given entries, used by
   *  `cmd-A` (select-all-visible). */
  selectAll: (entries: JobSelectionEntry[]) => void
  isSelected: (key: string) => boolean
  /** Number of currently-selected jobs. */
  selectedCount: () => number
  /** Distinct queues represented in the selection, used by the
   *  BulkActionBar to render the "across K queues" string AND to
   *  decide whether to disable cross-queue actions. */
  distinctQueues: () => Set<string>
  /** Group selected entries by queue id. Used by the
   *  cross-queue orchestrator to fan out one `bulkXxxJobs` Tauri
   *  call per queue + by the BulkActionBar's preview view to
   *  render the per-queue breakdown. */
  groupByQueue: () => Map<string, JobSelectionEntry[]>
}

export const useJobSelectionStore = create<JobSelectionStore>((set, get) => ({
  selection: new Map(),
  toggle: (entry) =>
    set((state) => {
      const next = new Map(state.selection)
      const k = jobSelectionKey(entry)
      if (next.has(k)) next.delete(k)
      else next.set(k, entry)
      return { selection: next }
    }),
  add: (entry) =>
    set((state) => {
      const next = new Map(state.selection)
      next.set(jobSelectionKey(entry), entry)
      return { selection: next }
    }),
  addRange: (entries) =>
    set((state) => {
      const next = new Map(state.selection)
      for (const e of entries) next.set(jobSelectionKey(e), e)
      return { selection: next }
    }),
  remove: (key) =>
    set((state) => {
      const next = new Map(state.selection)
      next.delete(key)
      return { selection: next }
    }),
  removeRange: (keys) =>
    set((state) => {
      const next = new Map(state.selection)
      for (const k of keys) next.delete(k)
      return { selection: next }
    }),
  clear: () => set({ selection: new Map(), requestedAction: null }),
  requestedAction: null,
  requestAction: (action) => set({ requestedAction: action }),
  selectAll: (entries) => {
    const next = new Map<string, JobSelectionEntry>()
    for (const e of entries) next.set(jobSelectionKey(e), e)
    set({ selection: next })
  },
  isSelected: (key) => get().selection.has(key),
  selectedCount: () => get().selection.size,
  distinctQueues: () => {
    const names = new Set<string>()
    for (const e of get().selection.values()) names.add(e.jobRef.queueId)
    return names
  },
  groupByQueue: () => {
    const groups = new Map<string, JobSelectionEntry[]>()
    for (const e of get().selection.values()) {
      const id = e.jobRef.queueId
      const arr = groups.get(id) ?? []
      arr.push(e)
      groups.set(id, arr)
    }
    return groups
  },
}))
