import { useNavigate } from "@tanstack/react-router"

import { ChevronDownIcon, ChevronRightIcon, PlusIcon, SearchIcon, XIcon } from "lucide-react"
import { useState } from "react"
import { Toolbar } from "@/components/data/Toolbar"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { useConnections } from "@/data/connections/queries"
import { useEnvironments } from "@/data/environments/queries"
import { useSetGroupCollapsed } from "@/data/groups/mutations"
import { useGroups } from "@/data/groups/queries"
import { useMcpSafetySettings } from "@/data/mcp/queries"
import { lastSelectedConnection } from "@/data/settings/settings"
import { useRefetchInterval } from "@/data/settings/useRefreshSettings"
import { useSetting } from "@/data/settings/useSetting"
import { buildSidebarTree, type GroupNode } from "@/features/connections"
import { Freshness } from "@/features/freshness"
import { Screen } from "@/layouts/Screen"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import { useSidebarUiStore } from "@/stores/useSidebarUiStore"
import { FleetTable, FleetTiles } from "./FleetTable"
import { countConnections, filterTree } from "./homeFilter"
import { HOME_HEALTH_FLOOR_MS } from "./useConnectionHealth"
import { type FleetRow, useFleet } from "./useFleet"

const EMPTY: never[] = []

/**
 * The home page: every connection, grouped, and the entry points: add a
 * connection, a group, set up agents.
 *
 * The fleet at a glance: totals (connections and how
 * many are offline, failed with the last hour, waiting, workers) above one
 * table per group: connection, health (latency and its trend), queues,
 * waiting, failed, workers, protection. An unreachable connection says why
 * in plain words, when the next try happens, and offers Retry now and Edit.
 *
 * Same data and same state as the sidebar: `list_groups` + `list_connections`
 * + `list_environments` through `buildSidebarTree`, and ONE persisted
 * `collapsed` flag per group: folding a group here folds it there.
 */
export function HomePage() {
  const openAddConnection = useSidebarUiStore((s) => s.openAddConnection)
  const requestNewGroup = useSidebarUiStore((s) => s.requestNewGroup)
  const [search, setSearch] = useState("")

  const connectionsQuery = useConnections()
  const { data: groupRows = [], isLoading: groupsLoading } = useGroups()
  const { data: environmentRows = [] } = useEnvironments()
  const { data: safety } = useMcpSafetySettings()
  // Fresh on every visit: the selection is written behind, without the cache.
  const lastOpened = useSetting(lastSelectedConnection, { staleTime: 0 })
  const setCollapsed = useSetGroupCollapsed()
  const fleet = useFleet(connectionsQuery.data ?? EMPTY)
  const healthInterval = useRefetchInterval(HOME_HEALTH_FLOOR_MS)

  if (connectionsQuery.isLoading || groupsLoading) {
    return (
      <div role="status" aria-label="Loading connections" className="grid gap-3 p-8 md:grid-cols-3">
        {[0, 1, 2].map((i) => (
          <Skeleton key={i} className="h-40 rounded-xl" />
        ))}
      </div>
    )
  }
  if (connectionsQuery.isError) {
    return (
      <div role="alert" className="m-8 rounded-md border border-(--danger)/40 bg-(--danger)/10 p-4">
        <p className="font-medium text-(--state-failed-text)">Could not load your connections</p>
        <p className="text-xs text-(--state-failed-text)/80">
          {connectionsQuery.error instanceof Error
            ? connectionsQuery.error.message
            : "Unknown error"}
        </p>
      </div>
    )
  }

  const connections = connectionsQuery.data ?? []
  if (connections.length === 0) {
    return <FirstRun onAdd={() => openAddConnection()} />
  }

  const tree = buildSidebarTree(groupRows, connections, environmentRows)
  const fleetById = new Map(fleet.map((r) => [r.connection.id, r]))
  const visible = filterTree(tree, search)
  const searching = search.trim() !== ""
  const shown = countConnections(visible)

  return (
    <Screen
      toolbar={
        <Toolbar
          title="Connections"
          meta={
            <span className="text-sm text-(--text2)" data-testid="home-count">
              {searching
                ? `${shown} of ${connections.length} ${connections.length === 1 ? "connection" : "connections"}`
                : `${connections.length} ${connections.length === 1 ? "connection" : "connections"} in ${tree.length} ${tree.length === 1 ? "group" : "groups"}`}
            </span>
          }
          actions={
            <>
              {connections.length > 0 ? (
                <Freshness queryKey={["queues"]} intervalMs={healthInterval} />
              ) : null}
              <div className="relative">
                <SearchIcon
                  className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-(--text2)"
                  aria-hidden="true"
                />
                <Input
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  placeholder="Search connections"
                  aria-label="Search connections"
                  className="w-64 pl-8 pr-8"
                  data-testid="home-search"
                />
                {searching ? (
                  <button
                    type="button"
                    onClick={() => setSearch("")}
                    aria-label="Clear search"
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-(--text2) hover:text-(--text)"
                  >
                    <XIcon className="size-4" aria-hidden="true" />
                  </button>
                ) : null}
              </div>
              <Button type="button" variant="default" size="sm" onClick={() => requestNewGroup()}>
                New group
              </Button>
              <Button
                variant="primary"
                type="button"
                size="sm"
                onClick={() => openAddConnection()}
                data-testid="home-add-connection"
                className="gap-1.5"
              >
                <PlusIcon className="size-4" aria-hidden="true" />
                Add connection
              </Button>
            </>
          }
        />
      }
      contentClassName="overflow-auto"
    >
      <div className="flex w-full flex-col gap-6" data-testid="home-page">
        <FleetTiles rows={fleet} />
        {visible.length === 0 ? (
          <div
            className="rounded-xl border border-dashed border-(--line) p-10 text-center"
            data-testid="home-no-match"
          >
            <p className="text-sm font-medium">No connection matches “{search.trim()}”.</p>
            <Button type="button" variant="ghost" size="sm" onClick={() => setSearch("")}>
              Clear search
            </Button>
          </div>
        ) : (
          visible.map((group) => (
            <GroupSection
              key={group.id}
              group={group}
              // While searching, everything that matches is shown.
              collapsed={group.collapsed && !searching}
              onToggle={() => setCollapsed.mutate({ id: group.id, collapsed: !group.collapsed })}
              onAdd={(environmentId) => openAddConnection({ groupId: group.id, environmentId })}
              lastOpenedId={lastOpened.value}
              readonlyDefault={safety?.readonlyDefault ?? false}
              fleet={fleetById}
            />
          ))
        )}
      </div>
    </Screen>
  )
}

function GroupSection({
  group,
  collapsed,
  onToggle,
  onAdd,
  lastOpenedId,
  readonlyDefault,
  fleet,
}: {
  group: GroupNode
  collapsed: boolean
  onToggle: () => void
  onAdd: (environmentId?: string) => void
  lastOpenedId: string | null
  readonlyDefault: boolean
  fleet: Map<string, FleetRow>
}) {
  const total = group.connections.length
  return (
    <section aria-label={group.name} data-testid="home-group" className="flex flex-col gap-3">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={!collapsed}
        className="flex items-center gap-2 self-start rounded-md text-left text-sm font-semibold uppercase tracking-wide text-(--text2) hover:text-(--text) focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)"
      >
        {collapsed ? (
          <ChevronRightIcon className="size-4" aria-hidden="true" />
        ) : (
          <ChevronDownIcon className="size-4" aria-hidden="true" />
        )}
        {group.name}
        <span className="rounded-full bg-(--surf) px-1.5 font-mono text-2xs font-normal tabular-nums">
          {total}
        </span>
      </button>

      {collapsed ? null : total === 0 ? (
        <p
          className="rounded-card border border-dashed border-(--line) px-4 py-3 text-sm text-(--text2)"
          data-testid="home-group-empty"
        >
          No connection in this group yet.{" "}
          <button
            type="button"
            className="text-(--text) underline underline-offset-2"
            onClick={() => onAdd()}
          >
            Add a connection here
          </button>
        </p>
      ) : (
        <FleetTable
          rows={group.connections.flatMap((c) => {
            const row = fleet.get(c.id)
            return row ? [row] : []
          })}
          lastOpenedId={lastOpenedId}
          readonlyDefault={readonlyDefault}
        />
      )}
    </section>
  )
}

/** No connection at all: what Ranch is, and the two ways in. */
function FirstRun({ onAdd }: { onAdd: () => void }) {
  const aiOn = useAiEnabled()
  const navigate = useNavigate()
  return (
    <div className="flex h-full items-center justify-center p-8" data-testid="home-first-run">
      <div className="flex max-w-md flex-col items-center gap-4 text-center">
        <img src="/ranch-logomark.png" alt="" className="size-14 rounded-xl" />
        <h1 className="text-2xl font-semibold tracking-tight">Welcome to Ranch</h1>
        <p className="text-sm text-(--text2)">
          {aiOn
            ? "A desktop client for your BullMQ queues: see what is waiting, running and failing, drill into a job, retry or clean up, and let your AI agents do the same, under your control."
            : "A client for your BullMQ queues: see what is waiting, running and failing, drill into a job, retry or clean up."}
        </p>
        <div className="flex gap-2">
          <Button
            variant="primary"
            type="button"
            onClick={onAdd}
            data-testid="home-add-first"
            className="gap-1.5"
          >
            <PlusIcon className="size-4" aria-hidden="true" />
            Add your first connection
          </Button>
          {aiOn ? (
            <Button
              type="button"
              variant="default"
              onClick={() => void navigate({ to: "/agents", search: { tab: "setup" } })}
            >
              Set up agents (MCP)
            </Button>
          ) : null}
        </div>
      </div>
    </div>
  )
}
