import { Link, useNavigate } from "@tanstack/react-router"
import { ArrowDownIcon, ArrowUpDownIcon, ArrowUpIcon, TriangleAlertIcon } from "lucide-react"
import { useMemo, useState } from "react"
import { useCapabilities } from "@/capabilities"
import { HeatStrip } from "@/components/data/charts"
import {
  DATA_TABLE_GRID,
  DataTable,
  type DataTableColumn,
  useColumnVisible,
} from "@/components/data/DataTable"
import { TargetMenu } from "@/components/menus/TargetMenu"
import { Skeleton } from "@/components/ui/skeleton"
import { useThroughput } from "@/data/metrics/queries"
import { useQueues, useQueueWorkerCounts } from "@/data/queues/queries"
import { formatCount, formatPercent } from "@/domain/format"
import { countOf, inLine } from "@/domain/stateCounts"
import { StaleNotice } from "@/features/freshness"
import type { JobState, Queue, QueueThroughput } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"
import { useQueueMenu } from "./useQueueMenu"

/**
 * The queues of a connection as a table to compare
 * them: status, what waits and runs, what failed, done per hour and the
 * failure rate (last hour, BullMQ metrics), the last 6 hours as heat strips
 * on one axis, workers. Every numeric column sorts; a zero is dim.
 */
type SortKey =
  | "name"
  | "waiting"
  | "active"
  | "delayed"
  | "failed"
  | "doneH"
  | "failRate"
  | "workers"

interface Row {
  queue: Queue
  waiting: number
  active: number
  delayed: number
  failed: number
  /** `null` = the queue records no metrics. */
  doneH: number | null
  failRate: number | null
  workers: number | null
  activity: QueueThroughput | undefined
  status: "paused" | "noWorker" | "working" | "idle"
}

const STATUS: Record<Row["status"], { label: string; className: string }> = {
  paused: { label: "Paused", className: "text-(--state-delayed-text)" },
  noWorker: { label: "No worker", className: "text-(--state-failed-text)" },
  working: { label: "Working", className: "text-(--state-active-text)" },
  idle: { label: "Idle", className: "text-(--text2)" },
}

const sum = (xs: readonly number[]) => xs.reduce((a, b) => a + b, 0)

export function QueuesTable({ connectionId }: { connectionId: string }) {
  const { data, isLoading, isError, error, dataUpdatedAt } = useQueues(connectionId)
  const { can, jobStates } = useCapabilities(connectionId)
  const counted = can("workers.count")
  const workers = useQueueWorkerCounts(connectionId, data, Boolean(data) && counted)
  const throughputOn = can("metrics.throughput")
  const lastHour = useThroughput(connectionId, "1h", { enabled: throughputOn }).data
  const sixHours = useThroughput(connectionId, "6h", { enabled: throughputOn }).data
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: "name", desc: false })

  const rows = useMemo<Row[]>(() => {
    const byId = (t: typeof lastHour) => new Map((t?.queues ?? []).map((q) => [q.queueId, q]))
    const hour = byId(lastHour)
    const six = byId(sixHours)
    return (data ?? []).map((queue) => {
      const c = (state: JobState) => countOf(queue.counts, state)
      const waiting = inLine(queue.counts, jobStates, true)
      const h = hour.get(queue.id)
      const done = h && !h.metricsOff ? sum(h.completed) : null
      const lost = h && !h.metricsOff ? sum(h.failed) : null
      const attached = workers?.get(queue.id) ?? null
      const status: Row["status"] = queue.isPaused
        ? "paused"
        : attached === 0 && waiting > 0
          ? "noWorker"
          : c("active") > 0
            ? "working"
            : "idle"
      return {
        queue,
        waiting,
        active: c("active"),
        delayed: c("delayed"),
        failed: c("failed"),
        doneH: done,
        failRate: done !== null && lost !== null && done + lost > 0 ? lost / (done + lost) : null,
        workers: attached,
        activity: six.get(queue.id),
        status,
      }
    })
  }, [data, workers, lastHour, sixHours, jobStates])

  const sorted = useMemo(() => {
    const dir = sort.desc ? -1 : 1
    return [...rows].sort((a, b) => {
      if (sort.key === "name") return dir * a.queue.name.localeCompare(b.queue.name)
      // Unknown values (metrics off, workers unknown) sort last either way.
      const x = a[sort.key]
      const y = b[sort.key]
      if (x === null && y === null) return a.queue.name.localeCompare(b.queue.name)
      if (x === null) return 1
      if (y === null) return -1
      return dir * (x - y) || a.queue.name.localeCompare(b.queue.name)
    })
  }, [rows, sort])

  if (isLoading) {
    return (
      <div className="flex flex-col gap-2" role="status" aria-label="Loading queues">
        {[0, 1, 2, 3].map((i) => (
          <Skeleton key={i} className="h-9 w-full" />
        ))}
      </div>
    )
  }
  // The error box only when there is nothing to show.
  if (isError && !data) {
    return (
      <div
        role="alert"
        className="flex items-start gap-2 rounded-card border border-(--danger)/40 bg-(--danger)/10 p-3 text-sm text-(--state-failed-text)"
      >
        <TriangleAlertIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
        <div>
          <p className="font-medium">Could not list queues</p>
          <p className="mt-0.5 text-xs opacity-80">
            {error instanceof Error ? error.message : "Unknown error"}
          </p>
        </div>
      </div>
    )
  }
  if (!data || data.length === 0) {
    return (
      <div className="rounded-card border border-(--line) bg-(--surf) p-4 text-sm text-(--text2)">
        <p className="font-medium text-(--text)">No BullMQ queues found.</p>
        <p className="mt-1 text-xs">
          Default prefix is <code className="font-mono">bull:</code>. Custom prefixes are
          auto-detected. If your queues still don&apos;t appear, ensure jobs have been added (BullMQ
          creates the <code className="font-mono">:meta</code> hash on first job).
        </p>
      </div>
    )
  }

  const header = (label: string, key: SortKey) => (
    <SortButton
      label={label}
      active={sort.key === key}
      desc={sort.desc}
      onClick={() =>
        setSort((s) => (s.key === key ? { key, desc: !s.desc } : { key, desc: key !== "name" }))
      }
      testId={`queues-sort-${key}`}
    />
  )
  const ariaSort = (key: SortKey) =>
    sort.key === key
      ? sort.desc
        ? ("descending" as const)
        : ("ascending" as const)
      : ("none" as const)
  // The always-on columns need about 700 px (tracks, gaps, padding); the
  // others come in when the table is wide enough for them too: the strip at
  // 830, Delayed at 920, Fail % at 1,000, never squeezed until they clip.
  const columns: DataTableColumn[] = [
    {
      id: "name",
      track: "minmax(9rem, 1.4fr)",
      header: header("Queue", "name"),
      ariaSort: ariaSort("name"),
    },
    { id: "status", track: "5.5rem", header: "Status" },
    {
      id: "waiting",
      track: "4.5rem",
      align: "end",
      header: header("Waiting", "waiting"),
      ariaSort: ariaSort("waiting"),
    },
    {
      id: "active",
      track: "4rem",
      align: "end",
      header: header("Active", "active"),
      ariaSort: ariaSort("active"),
    },
    {
      id: "delayed",
      track: "4.5rem",
      align: "end",
      header: header("Delayed", "delayed"),
      ariaSort: ariaSort("delayed"),
      minWidth: 920,
    },
    {
      id: "failed",
      track: "4rem",
      align: "end",
      header: header("Failed", "failed"),
      ariaSort: ariaSort("failed"),
    },
    {
      id: "doneH",
      track: "4.5rem",
      align: "end",
      header: header("Done/h", "doneH"),
      ariaSort: ariaSort("doneH"),
    },
    {
      id: "failRate",
      track: "4rem",
      align: "end",
      header: header("Fail %", "failRate"),
      ariaSort: ariaSort("failRate"),
      minWidth: 1000,
    },
    { id: "activity", track: "minmax(7rem, 1fr)", header: "Last 6 h", minWidth: 830 },
    {
      id: "workers",
      track: "4.5rem",
      align: "end",
      header: header("Workers", "workers"),
      ariaSort: ariaSort("workers"),
    },
  ]

  const paused = rows.filter((r) => r.status === "paused").length
  const failed = sum(rows.map((r) => r.failed))
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      {isError ? <StaleNotice error={error} updatedAt={dataUpdatedAt} what="the queues" /> : null}
      <p className="text-xs text-(--text2)" data-testid="queue-grid-summary">
        {data.length} {data.length === 1 ? "queue" : "queues"}
        {paused > 0 ? ` · ${paused} paused` : ""}
        {failed > 0 ? (
          <span className="text-(--state-failed-text)">
            {" "}
            · {failed.toLocaleString()} failed {failed === 1 ? "job" : "jobs"}
          </span>
        ) : null}
      </p>
      <DataTable
        rows={sorted}
        columns={columns}
        rowKey={(r) => `${r.queue.id}`}
        rowHeight={40}
        ariaLabel="BullMQ queues"
        testId="queues-table"
        renderRow={(row) => <QueueRow connectionId={connectionId} row={row} />}
      />
    </div>
  )
}

function SortButton({
  label,
  active,
  desc,
  onClick,
  testId,
}: {
  label: string
  active: boolean
  desc: boolean
  onClick: () => void
  testId: string
}) {
  const Icon = active ? (desc ? ArrowDownIcon : ArrowUpIcon) : ArrowUpDownIcon
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "inline-flex items-center gap-1 rounded-sm whitespace-nowrap uppercase outline-none hover:text-(--text) focus-visible:ring-2 focus-visible:ring-(--accent)/50",
        active && "text-(--text)",
      )}
      data-testid={testId}
    >
      {label}
      <Icon className={cn("size-3", !active && "opacity-40")} aria-hidden="true" />
    </button>
  )
}

const NUM = "font-tabular text-sm"

function Num({ value, tone }: { value: number | null; tone?: string }) {
  // Unknown is not zero: "-" stays readable (`--dim` is for zeros only).
  if (value === null) return <span className={cn(NUM, "text-(--text2)")}>-</span>
  return (
    <span className={cn(NUM, value === 0 ? "text-(--dim)" : (tone ?? "text-(--text)"))}>
      {formatCount(value)}
    </span>
  )
}

function QueueRow({ connectionId, row }: { connectionId: string; row: Row }) {
  const navigate = useNavigate()
  const menu = useQueueMenu(connectionId, row.queue)
  const name = row.queue.name
  const open = () =>
    void navigate({
      to: "/connections/$connectionId/queues/$queueId",
      params: { connectionId, queueId: row.queue.id },
    })
  const show = {
    delayed: useColumnVisible("delayed"),
    failRate: useColumnVisible("failRate"),
    activity: useColumnVisible("activity"),
  }
  return (
    <>
      <TargetMenu
        actions={menu.actions}
        render={
          // biome-ignore lint/a11y/useSemanticElements: ARIA-grid row of a virtualized table.
          <div
            role="row"
            tabIndex={0}
            onClick={open}
            onKeyDown={(e) => {
              if (e.key === "Enter") open()
            }}
            className={cn(
              DATA_TABLE_GRID,
              "min-h-10 cursor-pointer border-b border-(--line) px-3 py-1.5 hover:bg-(--surf2) focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-(--accent)/60",
            )}
            data-testid={`queue-row-${name}`}
          >
            <span className="flex min-w-0 flex-col">
              <Link
                to="/connections/$connectionId/queues/$queueId"
                params={{ connectionId, queueId: row.queue.id }}
                onClick={(e) => e.stopPropagation()}
                className="min-w-0 font-medium break-words hover:underline"
              >
                {name}
              </Link>
              <span className="font-tabular text-2xs wrap-anywhere text-(--text2)">
                {row.queue.id}
              </span>
            </span>
            <span
              className={cn("text-xs", STATUS[row.status].className)}
              data-testid={`queue-status-${name}`}
            >
              {STATUS[row.status].label}
            </span>
            <span className="text-right">
              <Num value={row.waiting} />
            </span>
            <span className="text-right">
              <Num value={row.active} tone="text-(--state-active-text)" />
            </span>
            {show.delayed ? (
              <span className="text-right">
                <Num value={row.delayed} />
              </span>
            ) : null}
            <span className="text-right">
              <Num value={row.failed} tone="text-(--state-failed-text)" />
            </span>
            <span
              className="text-right"
              title={row.doneH === null ? "This queue records no BullMQ metrics" : undefined}
            >
              <Num value={row.doneH} />
            </span>
            {show.failRate ? (
              <span
                className={cn(
                  "text-right",
                  NUM,
                  row.failRate === null
                    ? "text-(--text2)"
                    : row.failRate > 0
                      ? "text-(--state-failed-text)"
                      : "text-(--dim)",
                )}
              >
                {row.failRate === null ? "-" : formatPercent(row.failRate)}
              </span>
            ) : null}
            {show.activity ? (
              <HeatStrip
                completed={row.activity?.completed ?? []}
                failed={row.activity?.failed ?? []}
                metricsOff={row.activity?.metricsOff ?? false}
                label={`${name}, last 6 hours`}
              />
            ) : null}
            <span className="text-right">
              <Num
                value={row.workers}
                tone={row.status === "noWorker" ? "text-(--state-failed-text)" : undefined}
              />
            </span>
          </div>
        }
      />
      {menu.dialogs}
    </>
  )
}
