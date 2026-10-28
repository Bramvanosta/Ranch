import type { RecentItem } from "@/stores/useCommandPaletteStore"

import { rankBy } from "./fuzzyMatch"

export interface PaletteItem {
  kind: "connection" | "queue" | "settings" | "recent" | "group" | "action" | "job"
  id: string
  label: string
  hint?: string
  /** Route to navigate to. Absent for sidebar intents, which use `run`. */
  to?: string
  /** A sidebar intent (new group, reveal / rename a group…). Not added to recents. */
  run?: () => void
  /** What ⇥ offers for this result. */
  actions?: PaletteAction[]
}

/** One of the actions ⇥ lists for a result. */
export interface PaletteAction {
  id: string
  label: string
  run: () => void
}

export const SETTINGS_ACTIONS: PaletteItem[] = [
  // The home page: every connection, grouped, with live health.
  { kind: "settings", id: "go-home", label: "Go to Home: all connections", to: "/" },
  { kind: "settings", id: "go-settings", label: "Go to Settings", to: "/settings" },
  { kind: "settings", id: "go-audit", label: "Go to Audit log", to: "/audit" },
]

export const AGENTS_PAGE: PaletteItem = {
  kind: "settings",
  id: "go-agents",
  label: "Go to Agents",
  to: "/agents",
}

export type PaletteSections = Record<"Recent" | "Go to" | "Actions" | "Jobs", PaletteItem[]>

/**
 * The result list's sections. Empty search: recents, then Go to (the open
 * connection's queues, connections, groups, pages) and Actions. `#id`: the
 * jobs found. Anything else: fuzzy matches, where connections and queues
 * also match on their hint ("group / environment").
 */
export function paletteSections({
  allItems,
  recents,
  query,
  currentConnectionId,
  jobId,
  jobItems,
}: {
  allItems: PaletteItem[]
  recents: RecentItem[]
  query: string
  currentConnectionId: string | null
  jobId: string | null
  jobItems: PaletteItem[]
}): PaletteSections {
  const recentItems: PaletteItem[] = recents.map((r) => ({
    kind: "recent",
    id: `${r.kind}-${r.id}`,
    label: r.label,
    hint: r.hint,
    to: r.to,
  }))
  if (query === "") {
    const here = currentConnectionId
      ? allItems
          .filter((i) => i.kind === "queue" && i.id.startsWith(`${currentConnectionId}:`))
          .slice(0, 8)
      : []
    return {
      Recent: recentItems,
      "Go to": [
        ...here,
        ...allItems.filter((i) => i.kind === "connection"),
        ...allItems.filter((i) => i.kind === "group" && i.id.startsWith("go-group-")),
        ...allItems.filter((i) => i.kind === "settings"),
      ],
      Actions: allItems.filter((i) => i.kind === "action"),
      Jobs: [],
    }
  }
  if (jobId !== null) {
    return { Recent: [], "Go to": [], Actions: [], Jobs: jobItems }
  }
  const matchAll = rankBy(allItems, query, (i) =>
    i.kind === "connection" || i.kind === "queue" ? `${i.label} ${i.hint ?? ""}` : i.label,
  )
  return {
    Recent: rankBy(recentItems, query, (i) => i.label),
    "Go to": [
      ...matchAll.filter((i) => i.kind === "queue").slice(0, 8),
      ...matchAll.filter(
        (i) => i.kind === "connection" || i.kind === "group" || i.kind === "settings",
      ),
    ],
    Actions: matchAll.filter((i) => i.kind === "action"),
    Jobs: [],
  }
}
