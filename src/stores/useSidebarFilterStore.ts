import { create } from "zustand"

/** The sidebar filter text. */
interface SidebarFilterState {
  query: string
  setQuery: (query: string) => void
}

export const useSidebarFilterStore = create<SidebarFilterState>((set) => ({
  query: "",
  setQuery: (query) => set({ query }),
}))
