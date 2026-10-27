import { useMemo, useState } from "react"

import {
  DATA_TABLE_GRID,
  DataTable,
  type DataTableColumn,
  useColumnVisible,
} from "@/components/data/DataTable"
import { SectionHeader } from "@/components/data/SectionHeader"
import { Skeleton } from "@/components/ui/skeleton"
import { formatAge } from "@/features/jobs/formatAge"
import { agentLabel } from "@/features/mcp/agentLabel"
import type { AuditLogEntry, Job } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import { describeSchedule, runsOf } from "./describeSchedule"
import {
  DAY_MS,
  rowId,
  type SchedulerRow,
  useConnectionSchedulers,
  useRecentJobsOf,
  useSchedulerPausers,
} from "./useConnectionSchedulers"

/**
 * The schedulers of a connection: what will run in the
 * next 24 hours (one lane per scheduler, a dot per run; a paused one hatched
 * with who paused it), then a table — name and job, queue, schedule in plain
 * language, next run, last run with its outcome, state. A row opens the
 * scheduler in the inspector.
 */
export function SchedulersView({
  connectionId,
  selected,
  onSelect,
}: {
  connectionId: string
  selected: string | null
  onSelect: (row: SchedulerRow) => void
}) {
  const { rows, loading, error } = useConnectionSchedulers(connectionId)
  const pausers = useSchedulerPausers(connectionId)
  const queues = useMemo(() => {
    const seen = new Map(rows.map((r) => [r.queueRef.name, r.queueRef]))
    return [...seen.values()]
  }, [rows])
  const recent = useRecentJobsOf(connectionId, queues)
  const [now] = useState(() => Date.now())

  if (loading) {
    return (
      <div className="flex flex-col gap-2" role="status" aria-label="Loading schedulers">
        {[0, 1, 2].map((i) => (
          <Skeleton key={i} className="h-8 w-full" />
        ))}
      </div>
    )
  }
  if (error) {
    return (
      <p role="alert" className="text-sm text-(--state-failed-text)">
        Could not list the schedulers: {error.message}
      </p>
    )
  }
  if (rows.length === 0) {
    return (
      <p className="rounded-card border border-(--line) p-6 text-center text-sm text-(--text2)">
        No scheduler on this connection. A queue's Schedulers tab creates one.
      </p>
    )
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-5" data-testid="schedulers-view">
      <NextDay rows={rows} now={now} pausers={pausers} />
      <section aria-label="Schedulers" className="flex min-h-0 flex-1 flex-col gap-2">
        <SectionHeader title="Schedulers" count={rows.length} />
        <SchedulersTable
          rows={rows}
          now={now}
          recent={recent}
          selected={selected}
          onSelect={onSelect}
        />
      </section>
    </div>
  )
}

export function pausedBy(entry: AuditLogEntry | undefined, now: number): string {
  if (!entry) return "paused"
  const who = entry.source === "agent" ? (agentLabel(entry.agentClient) ?? "an agent") : "you"
  return `paused by ${who} · ${formatAge(Math.max(now - Date.parse(entry.createdAt), 0))} ago`
}

const HOURS = [0, 6, 12, 18, 24]

/** One lane per scheduler on the next 24 hours, a dot per run. */
function NextDay({
  rows,
  now,
  pausers,
}: {
  rows: SchedulerRow[]
  now: number
  pausers: Map<string, AuditLogEntry>
}) {
  const at = (t: number) => `${Math.min(Math.max(((t - now) / DAY_MS) * 100, 0), 100)}%`
  return (
    <section
      aria-label="Next 24 hours"
      className="flex flex-col gap-2"
      data-testid="schedulers-next-day"
    >
      <SectionHeader title="Next 24 hours" />
      <div className="grid grid-cols-[minmax(7rem,12rem)_minmax(0,1fr)] gap-x-3 gap-y-1.5 rounded-card border border-(--line) p-3">
        <span />
        <span className="relative h-4 text-2xs text-(--text2)" aria-hidden="true">
          {HOURS.map((h) => (
            <span
              key={h}
              className={cn(
                "absolute top-0 font-tabular whitespace-nowrap",
                h === 0 ? "left-0" : h === 24 ? "right-0" : "-translate-x-1/2",
              )}
              style={h === 0 || h === 24 ? undefined : { left: `${(h / 24) * 100}%` }}
            >
              {h === 0
                ? "now"
                : new Date(now + h * 3_600_000).toLocaleTimeString(undefined, {
                    hour: "2-digit",
                    minute: "2-digit",
                  })}
            </span>
          ))}
        </span>
        {rows.map((row) => {
          const id = rowId(row)
          const paused = row.scheduler.paused
          return (
            <div key={id} className="contents" data-testid="next-day-lane">
              <span className="min-w-0 text-xs break-words">{row.scheduler.key}</span>
              <span
                className={cn(
                  "relative flex h-4 items-center rounded-tag",
                  paused
                    ? "bg-[repeating-linear-gradient(135deg,var(--line)_0_2px,transparent_2px_6px)]"
                    : "bg-(--surf2)",
                )}
                role="img"
                aria-label={
                  paused
                    ? `${row.scheduler.key}: ${pausedBy(pausers.get(id), now)}`
                    : `${row.scheduler.key}: ${row.runs.length} runs in the next 24 hours`
                }
              >
                {paused ? (
                  <span className="px-2 text-2xs text-(--text2)" data-testid="next-day-paused">
                    {pausedBy(pausers.get(id), now)}
                  </span>
                ) : (
                  row.runs.map((t) => (
                    <span
                      key={t}
                      className="absolute size-1.5 -translate-x-1/2 rounded-full bg-(--accent)"
                      style={{ left: at(t) }}
                      data-testid="next-day-dot"
                    />
                  ))
                )}
              </span>
            </div>
          )
        })}
      </div>
    </section>
  )
}

const inWords = (ms: number) => {
  if (ms <= 0) return "due"
  const m = Math.round(ms / 60_000)
  if (m < 60) return `in ${m} min`
  const h = Math.floor(m / 60)
  if (h < 24) return `in ${h} h ${m % 60} min`
  return `in ${Math.floor(h / 24)} d ${h % 24} h`
}

function SchedulersTable({
  rows,
  now,
  recent,
  selected,
  onSelect,
}: {
  rows: SchedulerRow[]
  now: number
  recent: Map<string, Job[]>
  selected: string | null
  onSelect: (row: SchedulerRow) => void
}) {
  const columns: DataTableColumn[] = [
    { id: "name", track: "minmax(9rem, 1.4fr)", header: "Scheduler" },
    { id: "queue", track: "minmax(5rem, 0.6fr)", header: "Queue" },
    { id: "schedule", track: "minmax(9rem, 1.2fr)", header: "Schedule" },
    { id: "next", track: "7rem", header: "Next" },
    { id: "last", track: "minmax(7rem, 0.8fr)", header: "Last run", minWidth: 760 },
    { id: "state", track: "4.5rem", header: "State" },
  ]
  return (
    <DataTable
      rows={rows}
      columns={columns}
      rowKey={rowId}
      rowHeight={44}
      ariaLabel="Schedulers"
      testId="schedulers-table"
      renderRow={(row) => (
        <SchedulerTableRow
          row={row}
          now={now}
          last={runsOf(row.scheduler, recent.get(row.queueRef.name) ?? [])[0]}
          selected={selected === rowId(row)}
          onSelect={() => onSelect(row)}
        />
      )}
    />
  )
}

function SchedulerTableRow({
  row,
  now,
  last,
  selected,
  onSelect,
}: {
  row: SchedulerRow
  now: number
  last: Job | undefined
  selected: boolean
  onSelect: () => void
}) {
  const s = row.scheduler
  const { main, plain } = describeSchedule(s)
  const showLast = useLastColumn()
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
        "min-h-11 cursor-pointer border-b border-(--line) px-3 py-1.5 text-sm hover:bg-(--surf2) focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-(--accent)/60",
        selected && "bg-(--sel)",
      )}
      data-testid={`scheduler-row-${s.key}`}
    >
      <span className="flex min-w-0 flex-col">
        <span className="font-medium break-words">{s.key}</span>
        <span className="text-xs break-words text-(--text2)">→ {s.name}</span>
      </span>
      <span className="min-w-0 break-words text-(--text2)">{row.queueRef.name}</span>
      <span className="flex min-w-0 flex-col">
        <span className="font-tabular text-xs break-all">{main}</span>
        {plain ? <span className="text-xs text-(--text2)">{plain}</span> : null}
      </span>
      <span className="flex flex-col">
        {s.paused ? (
          <span className="text-(--text2)">—</span>
        ) : (
          <>
            <span className={cn("text-xs", s.nextRun < now && "text-(--state-delayed-text)")}>
              {inWords(s.nextRun - now)}
            </span>
            <span className="font-tabular text-2xs text-(--text2)">
              {new Date(s.nextRun).toLocaleTimeString(undefined, {
                hour: "2-digit",
                minute: "2-digit",
              })}
            </span>
          </>
        )}
      </span>
      {showLast ? (
        <span
          className="flex min-w-0 items-center gap-1.5 text-xs"
          data-testid="scheduler-last-run"
        >
          {last ? (
            <>
              <span
                className={cn(
                  "size-2 shrink-0 rounded-full",
                  last.state === "failed"
                    ? "bg-(--state-failed)"
                    : last.state === "completed"
                      ? "bg-(--state-completed)"
                      : "bg-(--state-active)",
                )}
                aria-hidden="true"
              />
              <span className="min-w-0 break-words">
                {last.state} ·{" "}
                {formatAge(
                  Math.max(now - (last.finishedOn ?? last.processedOn ?? last.timestamp), 0),
                )}{" "}
                ago
              </span>
            </>
          ) : (
            <span className="text-(--text2)">
              {s.iterationCount > 0 ? "not in the recent jobs" : "never"}
            </span>
          )}
        </span>
      ) : null}
      <span className={cn("text-xs", s.paused ? "text-(--state-delayed-text)" : "text-(--text2)")}>
        {s.paused ? "Paused" : "Active"}
      </span>
    </div>
  )
}

function useLastColumn() {
  return useColumnVisible("last")
}
