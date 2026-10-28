import { Link, useNavigate } from "@tanstack/react-router"
import { SearchIcon, XIcon } from "lucide-react"
import { useRef } from "react"

import { EnvDot } from "@/components/data/Env"
import { Kbd } from "@/components/data/Kbd"
import { useGroups } from "@/data/groups/queries"
import { useCommandPaletteStore } from "@/stores/useCommandPaletteStore"
import { useSidebarFilterStore } from "@/stores/useSidebarFilterStore"

import { filterQueues, highlightParts, looksLikeJobId } from "./sidebarFilter"
import { useAllConnectionQueues } from "./useAttentionCount"

/** What the user typed, filtered across every connection. */
function useFilterResults() {
  const query = useSidebarFilterStore((s) => s.query)
  const { data: groups = [] } = useGroups()
  const { connections, queuesOf } = useAllConnectionQueues()
  return { query, ...filterQueues(query, groups, connections, queuesOf) }
}

/**
 * The sidebar's search field: typing filters the tree live across
 * connections; ↓ moves into the matches; Enter opens the first one, or hands
 * a job ID (or a query with no match) to the palette; Esc clears.
 */
export function SidebarFilterField() {
  const query = useSidebarFilterStore((s) => s.query)
  const setQuery = useSidebarFilterStore((s) => s.setQuery)
  const openPalette = useCommandPaletteStore((s) => s.openWith)
  const navigate = useNavigate()
  const results = useFilterResults()
  const inputRef = useRef<HTMLInputElement>(null)

  function onKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Escape") {
      e.preventDefault()
      setQuery("")
      return
    }
    if (e.key === "ArrowDown") {
      e.preventDefault()
      document
        .querySelector<HTMLElement>("[data-sidebar-filter-results] [data-sidebar-row]")
        ?.focus()
      return
    }
    if (e.key === "Enter" && query.trim() !== "") {
      e.preventDefault()
      const first = results.groups[0]?.connections[0]
      if (looksLikeJobId(query) || !first) {
        openPalette(query.trim())
        return
      }
      const queue = first.queues[0]
      if (queue) {
        void navigate({
          to: "/connections/$connectionId/queues/$queueId",
          params: { connectionId: first.connection.id, queueId: queue.id },
        })
      }
    }
  }

  return (
    <div className="relative shrink-0 px-2 pt-2">
      <SearchIcon
        aria-hidden="true"
        className="pointer-events-none absolute top-1/2 left-4.5 mt-1 size-3.5 -translate-y-1/2 text-(--text2)"
      />
      <input
        ref={inputRef}
        type="search"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={onKeyDown}
        placeholder="Filter queues"
        aria-label="Filter queues across connections"
        data-testid="sidebar-filter"
        className="h-control w-full rounded-control border border-(--line) bg-(--field) pr-12 pl-7 text-sm text-(--text) outline-none placeholder:text-(--text2) focus-visible:border-(--accent) focus-visible:ring-3 focus-visible:ring-(--accent)/50 [&::-webkit-search-cancel-button]:hidden"
      />
      {query ? (
        <button
          type="button"
          aria-label="Clear the filter"
          onClick={() => {
            setQuery("")
            inputRef.current?.focus()
          }}
          className="absolute top-1/2 right-4 mt-1 -translate-y-1/2 text-(--text2) hover:text-(--text)"
        >
          <XIcon aria-hidden="true" className="size-3.5" />
        </button>
      ) : (
        <Kbd
          label="Command K opens the palette"
          className="absolute top-1/2 right-4 mt-1 -translate-y-1/2"
        >
          ⌘K
        </Kbd>
      )}
    </div>
  )
}

/** The filtered tree that replaces the sidebar's while the field holds text. */
export function SidebarFilterResults() {
  const { query, groups, queueCount, connectionCount } = useFilterResults()
  const setQuery = useSidebarFilterStore((s) => s.setQuery)
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: Esc anywhere in the results clears the filter
    <div
      data-sidebar-filter-results
      className="flex flex-col gap-1 px-1 py-1"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault()
          setQuery("")
          document.querySelector<HTMLElement>("[data-testid=sidebar-filter]")?.focus()
        }
      }}
    >
      <p
        className="px-2 py-1 text-xs text-(--text2)"
        role="status"
        data-testid="sidebar-filter-count"
      >
        {queueCount === 0
          ? `No queue matches “${query.trim()}”. Enter searches everywhere.`
          : `${queueCount} ${queueCount === 1 ? "queue" : "queues"} in ${connectionCount} ${connectionCount === 1 ? "connection" : "connections"}`}
      </p>
      <ul className="flex flex-col gap-1">
        {groups.map((group) => (
          <li key={group.groupId}>
            <p className="px-2 py-1 text-2xs font-semibold tracking-wide text-(--text2) uppercase">
              {group.groupName}
            </p>
            <ul>
              {group.connections.map(({ connection, queues }) => (
                <li key={connection.id}>
                  <p className="flex items-center gap-2 px-2 py-1 text-sm text-(--text)">
                    <EnvDot color={connection.envColor} label={connection.envLabel} />
                    <span className="min-w-0 wrap-anywhere">{connection.name}</span>
                  </p>
                  <ul>
                    {queues.map((q) => {
                      const [before, match, after] = highlightParts(q.name, query)
                      return (
                        <li key={q.name}>
                          <Link
                            to="/connections/$connectionId/queues/$queueId"
                            params={{ connectionId: connection.id, queueId: q.id }}
                            data-sidebar-row
                            className="flex min-h-row-queue items-center rounded-control py-1 pr-2 pl-7 text-sm wrap-anywhere text-(--text) outline-none hover:bg-(--surf2) focus-visible:ring-2 focus-visible:ring-(--accent)/50"
                          >
                            <span>
                              {before}
                              {match ? (
                                <mark className="rounded-[3px] bg-(--accent)/25 text-(--text)">
                                  {match}
                                </mark>
                              ) : null}
                              {after}
                            </span>
                          </Link>
                        </li>
                      )
                    })}
                  </ul>
                </li>
              ))}
            </ul>
          </li>
        ))}
      </ul>
    </div>
  )
}
