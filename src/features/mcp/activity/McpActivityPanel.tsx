import { useState } from "react"
import { Segmented } from "@/components/data/Segmented"
import { ViewPanel, ViewTabs, ViewTabsRoot } from "@/components/data/ViewTabs"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { JsonViewer } from "@/features/jobs/JsonViewer"
import { formatDurationMs } from "@/features/metrics/format"
import type { ActivityEvent, ActivityFilter } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import { ActivityStatusChip } from "./ActivityStatusChip"
import {
  ACTIVITY_PAGE_SIZE,
  EMPTY_FILTER,
  useActivityEnabled,
  useClearMcpActivity,
  useExportMcpActivity,
  useMcpActivity,
  useMcpActivitySessions,
} from "./useMcpActivity"

function formatTime(iso: string): string {
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString()
}

/**
 * Agents › Activity (`McpActivityPanel`). Everything
 * agents did through Ranch, reads included: events and sessions, filters,
 * detail, export, clear. Distinct from the audit log, which only holds
 * destructive operations and can never be cleared.
 */
export function McpActivityPanel() {
  const [filter, setFilter] = useState<ActivityFilter>(EMPTY_FILTER)
  const [page, setPage] = useState(0)
  const [tab, setTab] = useState("events")
  const [openId, setOpenId] = useState<string | null>(null)

  const activity = useMcpActivity(filter, page)
  const sessions = useMcpActivitySessions(tab === "sessions")
  const { enabled, setEnabled } = useActivityEnabled()
  const clear = useClearMcpActivity()
  const exportActivity = useExportMcpActivity()

  function patchFilter(patch: Partial<ActivityFilter>) {
    setFilter((f) => ({ ...f, ...patch }))
    setPage(0)
  }

  const events = activity.data?.events ?? []
  const total = activity.data?.total ?? 0
  const lastPage = Math.max(0, Math.ceil(total / ACTIVITY_PAGE_SIZE) - 1)

  return (
    <div className="flex flex-col gap-3" data-testid="mcp-activity-panel">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-2 text-sm">
          <Switch
            id="mcp-activity-enabled"
            checked={enabled}
            onCheckedChange={(checked) => setEnabled(checked)}
          />
          <label htmlFor="mcp-activity-enabled">Record agent activity</label>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="default"
            size="sm"
            disabled={exportActivity.isPending}
            onClick={() => exportActivity.mutate({ filter, format: "json" })}
          >
            Export JSON
          </Button>
          <Button
            variant="default"
            size="sm"
            disabled={exportActivity.isPending}
            onClick={() => exportActivity.mutate({ filter, format: "csv" })}
          >
            Export CSV
          </Button>
          <Button
            variant="ghost"
            size="sm"
            disabled={clear.isPending || total === 0}
            onClick={() => clear.mutate()}
          >
            Clear
          </Button>
        </div>
      </div>

      <ViewTabsRoot value={tab} onValueChange={setTab}>
        <ViewTabs
          className="px-0"
          items={[
            { value: "events", label: "Events" },
            { value: "sessions", label: "Sessions" },
          ]}
        />

        <ViewPanel value="events" className="flex flex-col gap-2">
          <div className="flex flex-wrap items-center gap-2">
            {/* The three questions asked of this log. */}
            <Segmented
              label="Show"
              value={
                filter.status === "error" ? "errors" : filter.kind === "write" ? "writes" : "all"
              }
              onChange={(v) =>
                patchFilter(
                  v === "writes"
                    ? { kind: "write", status: null }
                    : v === "errors"
                      ? { kind: null, status: "error" }
                      : { kind: null, status: null },
                )
              }
              options={[
                { value: "all", label: "All" },
                { value: "writes", label: "Writes" },
                { value: "errors", label: "Errors" },
              ]}
            />
            {filter.sessionId ? (
              <Button
                variant="default"
                size="sm"
                className="h-8 text-xs"
                onClick={() => patchFilter({ sessionId: null })}
              >
                Session {filter.sessionId.slice(-6)} ✕
              </Button>
            ) : null}
            <span className="ml-auto text-xs text-(--text2)">
              {total} event{total === 1 ? "" : "s"}
            </span>
          </div>

          {activity.isError ? (
            <p className="rounded-md border border-(--danger)/40 bg-(--danger)/10 px-3 py-2 text-sm text-(--state-failed-text)">
              Could not load the activity log.
            </p>
          ) : events.length === 0 ? (
            <p className="px-1 py-6 text-center text-sm text-(--text2)">
              {enabled
                ? "Nothing yet. Calls made by agents through the Ranch MCP server will appear here."
                : "Recording is off."}
            </p>
          ) : (
            <ul className="divide-y divide-(--line) rounded-md border border-(--line)">
              {events.map((event) => (
                <EventRow
                  key={event.id}
                  event={event}
                  open={openId === event.id}
                  onToggle={() => setOpenId(openId === event.id ? null : event.id)}
                  onSession={() => patchFilter({ sessionId: event.sessionId })}
                />
              ))}
            </ul>
          )}

          {lastPage > 0 ? (
            <div className="flex items-center justify-end gap-2 text-xs">
              <Button
                variant="default"
                size="sm"
                disabled={page === 0}
                onClick={() => setPage(page - 1)}
              >
                Newer
              </Button>
              <span className="text-(--text2)">
                {page + 1} / {lastPage + 1}
              </span>
              <Button
                variant="default"
                size="sm"
                disabled={page >= lastPage}
                onClick={() => setPage(page + 1)}
              >
                Older
              </Button>
            </div>
          ) : null}
        </ViewPanel>

        <ViewPanel value="sessions">
          {(sessions.data ?? []).length === 0 ? (
            <p className="px-1 py-6 text-center text-sm text-(--text2)">No sessions yet.</p>
          ) : (
            <ul className="divide-y divide-(--line) rounded-md border border-(--line)">
              {(sessions.data ?? []).map((s) => (
                <li key={s.sessionId} className="flex items-center gap-3 px-3 py-2 text-sm">
                  <div className="min-w-0 flex-1">
                    <p className="truncate font-medium">{s.clientHint ?? "Unknown client"}</p>
                    <p className="text-xs text-(--text2)">
                      {formatTime(s.startedAt)} → {formatTime(s.lastAt)}
                    </p>
                  </div>
                  <span className="text-xs text-(--text2)">
                    {s.total} call{s.total === 1 ? "" : "s"} · {s.writes} write
                    {s.writes === 1 ? "" : "s"}
                    {s.notSuccessful > 0 ? ` · ${s.notSuccessful} not successful` : ""}
                  </span>
                  <Button
                    variant="default"
                    size="sm"
                    className="h-7 text-xs"
                    onClick={() => {
                      patchFilter({ sessionId: s.sessionId })
                      setTab("events")
                    }}
                  >
                    View
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </ViewPanel>
      </ViewTabsRoot>
    </div>
  )
}

function EventRow({
  event,
  open,
  onToggle,
  onSession,
}: {
  event: ActivityEvent
  open: boolean
  onToggle: () => void
  onSession: () => void
}) {
  let target: unknown = null
  if (event.targetJson) {
    try {
      target = JSON.parse(event.targetJson)
    } catch {
      target = event.targetJson
    }
  }
  return (
    <li data-testid={`activity-event-${event.id}`}>
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className="flex w-full items-center gap-3 px-3 py-2 text-left text-sm hover:bg-(--surf)/40"
      >
        <span className="w-36 shrink-0 font-tabular text-xs text-(--text2)">
          {formatTime(event.timestamp)}
        </span>
        <span
          className={cn(
            "w-12 shrink-0 text-2xs font-semibold uppercase tracking-wide",
            event.kind === "write" ? "text-(--text)" : "text-(--text2)",
          )}
        >
          {event.kind}
        </span>
        <span className="min-w-0 flex-1 truncate">
          <span className="font-mono text-xs">{event.tool}</span>
          {event.connectionName ? (
            <span className="text-(--text2)"> · {event.connectionName}</span>
          ) : null}
        </span>
        <ActivityStatusChip status={event.status} />
        <span className="w-14 shrink-0 text-right font-tabular text-xs text-(--text2)">
          {formatDurationMs(event.durationMs)}
        </span>
      </button>
      {open ? (
        <div className="flex flex-col gap-2 border-t border-(--line) bg-(--surf)/20 px-3 py-2 text-xs">
          <p>
            <span className="text-(--text2)">Client:</span> {event.clientHint ?? "unknown"} ·{" "}
            <button type="button" className="underline" onClick={onSession}>
              this session only
            </button>
          </p>
          {event.error ? (
            <p className="whitespace-pre-wrap break-words text-(--state-failed-text)">
              {event.error}
            </p>
          ) : null}
          {event.approvalId ? (
            <p>
              <span className="text-(--text2)">Approval:</span>{" "}
              <span className="font-mono">{event.approvalId}</span>
            </p>
          ) : null}
          {target !== null ? (
            <JsonViewer value={target} copyLabel="arguments" collapseDeeperThan={3} />
          ) : null}
        </div>
      ) : null}
    </li>
  )
}
