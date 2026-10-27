import { useQuery } from "@tanstack/react-query"
import { useMemo, useState } from "react"

import {
  DATA_TABLE_GRID,
  DataTable,
  type DataTableColumn,
  useColumnVisible,
} from "@/components/data/DataTable"
import { Segmented } from "@/components/data/Segmented"
import { Toolbar } from "@/components/data/Toolbar"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { Screen } from "@/layouts/Screen"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import {
  type AuditAction,
  type AuditLogEntry,
  type AuditSource,
  commands,
} from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import { AuditEntryDetail } from "./AuditEntryDetail"
import { AUDIT_ACTIONS } from "./auditConstants"
import { ACTION_LABELS, formatWhen, whoOf } from "./auditLabels"
import { formatAuditTarget } from "./formatAuditTarget"
import { useAuditExport } from "./useAuditExport"
import { useAuditLog } from "./useAuditLog"

/**
 * The audit log, as a table: time, who,
 * action, target, where, result. Filters: text (over everything shown),
 * actor (Everyone / You / Agents when agents exist), connection, action;
 * Export as CSV or JSON. A row opens its details beside the table.
 *
 * Connection, action and actor filter server-side (`list_audit_log_entries`);
 * the text filter runs over the page loaded.
 */
type Actor = "everyone" | "you" | "agents"

type FilterState = {
  text: string
  actor: Actor
  connectionId: string
  action: "" | AuditAction
}

const EMPTY_FILTER: FilterState = { text: "", actor: "everyone", connectionId: "", action: "" }

export function AuditLogPage() {
  const aiOn = useAiEnabled()
  const [selected, setSelected] = useState<AuditLogEntry | null>(null)
  return (
    <Screen
      toolbar={
        <Toolbar
          title="Audit log"
          meta={
            <span className="text-sm text-(--text2)">
              Every destructive operation, by you{aiOn ? " or by an agent" : ""}.
            </span>
          }
        />
      }
      inspector={
        selected ? (
          <AuditEntryDetail entry={selected} onClose={() => setSelected(null)} />
        ) : undefined
      }
    >
      <AuditLogViewer selectedId={selected?.id ?? null} onSelect={setSelected} />
    </Screen>
  )
}

export function AuditLogViewer({
  selectedId = null,
  onSelect = () => {},
}: {
  selectedId?: string | null
  onSelect?: (entry: AuditLogEntry) => void
}) {
  const [filter, setFilter] = useState<FilterState>(EMPTY_FILTER)

  const connectionsQuery = useQuery({
    queryKey: ["connections"],
    queryFn: async () => {
      const r = await commands.listConnections()
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    staleTime: 30_000,
  })

  const args = useMemo(
    () => ({
      connectionId: filter.connectionId || undefined,
      action: (filter.action || undefined) as AuditAction | undefined,
      source: (filter.actor === "you" ? "ui" : filter.actor === "agents" ? "agent" : undefined) as
        | AuditSource
        | undefined,
    }),
    [filter.connectionId, filter.action, filter.actor],
  )

  const { data, isLoading, isError, error, refetch, isFetching } = useAuditLog(args)
  const exportMutation = useAuditExport()
  const rows = useMemo(() => {
    const needle = filter.text.trim().toLowerCase()
    if (!needle) return data ?? []
    return (data ?? []).filter((e) =>
      [
        formatAuditTarget(e.target),
        ACTION_LABELS[e.action],
        whoOf(e),
        e.connectionName ?? "",
        e.environmentName ?? "",
        e.outcomeReason ?? "",
      ]
        .join(" ")
        .toLowerCase()
        .includes(needle),
    )
  }, [data, filter.text])

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3" data-testid="audit-log-viewer">
      <FilterBar
        filter={filter}
        onFilterChange={setFilter}
        connections={connectionsQuery.data ?? []}
        onExport={(format) => exportMutation.mutate({ ...args, format })}
        exportPending={exportMutation.isPending}
      />
      {isLoading ? (
        <AuditSkeleton />
      ) : isError ? (
        <div
          role="alert"
          className="flex flex-col gap-2 rounded-card border border-(--danger)/40 bg-(--danger)/10 p-3 text-sm text-(--state-failed-text)"
        >
          <p className="font-medium">Could not load audit log</p>
          <p className="text-xs">{error instanceof Error ? error.message : "Unknown error"}</p>
          <Button variant="default" size="sm" onClick={() => refetch()}>
            Retry
          </Button>
        </div>
      ) : !data || data.length === 0 ? (
        <div
          className="flex flex-col items-center justify-center gap-2 rounded-card border border-dashed border-(--line) p-8 text-center"
          data-testid="audit-log-empty"
        >
          <p className="text-sm font-medium">No audit entries yet.</p>
          <p className="text-xs text-(--text2)">
            Retries, removals, pauses, unlocks and every other change land here.
          </p>
        </div>
      ) : (
        <AuditTable
          entries={rows}
          isFetching={isFetching}
          selectedId={selectedId}
          onSelect={onSelect}
        />
      )}
    </div>
  )
}

function FilterBar({
  filter,
  onFilterChange,
  connections,
  onExport,
  exportPending,
}: {
  filter: FilterState
  onFilterChange: (next: FilterState) => void
  connections: Array<{ id: string; name: string; envLabel?: string }>
  onExport: (format: "json" | "csv") => void
  exportPending: boolean
}) {
  const aiOn = useAiEnabled()
  const set = <K extends keyof FilterState>(k: K, v: FilterState[K]) =>
    onFilterChange({ ...filter, [k]: v })
  const touched =
    filter.text !== "" ||
    filter.actor !== "everyone" ||
    filter.connectionId !== "" ||
    filter.action !== ""

  return (
    <div className="flex flex-wrap items-center gap-2" data-testid="audit-filter-bar">
      <Input
        value={filter.text}
        onChange={(e) => set("text", e.target.value)}
        placeholder="Filter by text"
        aria-label="Filter by text"
        className="h-8 w-56 text-sm"
        data-testid="audit-filter-text"
      />
      <Segmented
        label="Who"
        value={filter.actor}
        onChange={(v) => set("actor", v)}
        options={[
          { value: "everyone", label: "Everyone", testId: "audit-actor-everyone" },
          { value: "you", label: "You", testId: "audit-actor-you" },
          ...(aiOn
            ? [{ value: "agents" as const, label: "Agents", testId: "audit-actor-agents" }]
            : []),
        ]}
      />
      <select
        value={filter.connectionId}
        onChange={(e) => set("connectionId", e.target.value)}
        aria-label="Connection"
        className="h-8 rounded-control border border-(--line) bg-(--field) px-2 text-sm"
        data-testid="audit-filter-connection"
      >
        <option value="">All connections</option>
        {connections.map((c) => (
          <option key={c.id} value={c.id}>
            {c.envLabel ? `${c.name} (${c.envLabel})` : c.name}
          </option>
        ))}
      </select>
      <select
        value={filter.action}
        onChange={(e) => set("action", e.target.value as FilterState["action"])}
        aria-label="Action"
        className="h-8 rounded-control border border-(--line) bg-(--field) px-2 text-sm"
        data-testid="audit-filter-action"
      >
        <option value="">All actions</option>
        {AUDIT_ACTIONS.map((a) => (
          <option key={a} value={a}>
            {ACTION_LABELS[a]}
          </option>
        ))}
      </select>
      <div className="ml-auto flex items-center gap-1.5">
        {touched ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onFilterChange(EMPTY_FILTER)}
            data-testid="audit-filter-clear"
          >
            Clear
          </Button>
        ) : null}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="default"
              size="sm"
              disabled={exportPending}
              data-testid="audit-export-trigger"
            >
              Export
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent>
            <DropdownMenuItem onClick={() => onExport("csv")} data-testid="audit-export-csv">
              Export CSV
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => onExport("json")} data-testid="audit-export-json">
              Export JSON
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  )
}

const RESULT: Record<AuditLogEntry["outcome"], { label: string; className: string }> = {
  success: { label: "Done", className: "text-(--state-completed-text)" },
  failure: { label: "Failed", className: "text-(--state-failed-text)" },
  cancelled: { label: "Cancelled", className: "text-(--text2)" },
}

function AuditTable({
  entries,
  isFetching,
  selectedId,
  onSelect,
}: {
  entries: AuditLogEntry[]
  isFetching: boolean
  selectedId: string | null
  onSelect: (entry: AuditLogEntry) => void
}) {
  const columns: DataTableColumn[] = [
    { id: "time", track: "8.5rem", header: "Time" },
    { id: "who", track: "minmax(5rem, 0.7fr)", header: "Who" },
    { id: "action", track: "minmax(7rem, 0.8fr)", header: "Action" },
    { id: "target", track: "minmax(8rem, 1.4fr)", header: "Target" },
    { id: "where", track: "minmax(7rem, 1fr)", header: "Where", minWidth: 760 },
    { id: "result", track: "minmax(5rem, 0.8fr)", header: "Result" },
  ]
  return (
    <DataTable
      rows={entries}
      columns={columns}
      rowKey={(e) => e.id}
      rowHeight={40}
      ariaLabel="Audit log"
      testId="audit-log-table"
      className={isFetching ? "opacity-90" : undefined}
      renderRow={(entry) => (
        <AuditRow
          entry={entry}
          selected={entry.id === selectedId}
          onSelect={() => onSelect(entry)}
        />
      )}
    />
  )
}

function AuditRow({
  entry,
  selected,
  onSelect,
}: {
  entry: AuditLogEntry
  selected: boolean
  onSelect: () => void
}) {
  const aiOn = useAiEnabled()
  const showWhere = useColumnVisible("where")
  const result = RESULT[entry.outcome]
  return (
    // biome-ignore lint/a11y/useSemanticElements: ARIA-grid row of a virtualized table.
    <div
      role="row"
      tabIndex={0}
      aria-selected={selected}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault()
          onSelect()
        }
      }}
      className={cn(
        DATA_TABLE_GRID,
        "min-h-10 cursor-pointer border-b border-(--line) px-3 py-1.5 text-sm hover:bg-(--surf2) focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-(--accent)/60",
        selected && "bg-(--sel)",
      )}
      data-testid={`audit-row-${entry.id}`}
    >
      <span
        className="font-tabular text-xs text-(--text2)"
        title={entry.createdAt}
        data-testid="audit-row-time"
      >
        {formatWhen(entry.createdAt)}
      </span>
      <span
        className={cn("min-w-0 break-words", aiOn && entry.source === "agent" && "text-(--ai)")}
        data-testid="audit-row-source"
        data-source={entry.source}
      >
        {aiOn && entry.source === "agent" ? <span aria-hidden="true">✦ </span> : null}
        {whoOf(entry)}
      </span>
      <span className="min-w-0 break-words" data-testid="audit-row-action">
        {ACTION_LABELS[entry.action]}
      </span>
      <span className="min-w-0 font-tabular text-xs break-all" data-testid="audit-row-target">
        {formatAuditTarget(entry.target)}
      </span>
      {showWhere ? (
        <span className="flex min-w-0 items-center gap-1.5 text-xs text-(--text2)">
          <span
            aria-hidden="true"
            className="h-3 w-1 shrink-0 rounded-sm"
            style={{ backgroundColor: entry.envColor ? `var(--${entry.envColor})` : "transparent" }}
            data-testid="audit-row-env-stripe"
          />
          <span className="min-w-0 break-words">
            {[entry.environmentName, entry.connectionName].filter(Boolean).join(" / ") || "—"}
          </span>
        </span>
      ) : null}
      <span
        className={cn("min-w-0 text-xs break-words", result.className)}
        data-outcome={entry.outcome}
        data-testid="audit-row-outcome"
      >
        {result.label}
        {entry.outcomeReason && entry.outcome !== "success" ? ` · ${entry.outcomeReason}` : ""}
      </span>
    </div>
  )
}
function AuditSkeleton() {
  return (
    <div
      role="status"
      aria-label="Loading audit log"
      className="flex flex-col gap-1 rounded-md border border-(--line)/40 p-2"
    >
      {[0, 1, 2, 3, 4].map((i) => (
        <div key={i} className="flex items-center gap-3 px-1 py-2">
          <Skeleton className="size-4 shrink-0" />
          <Skeleton className="h-4 w-32 shrink-0" />
          <Skeleton className="h-4 w-16 shrink-0" />
          <Skeleton className="h-4 flex-1" />
          <Skeleton className="h-4 w-6 shrink-0" />
        </div>
      ))}
    </div>
  )
}
