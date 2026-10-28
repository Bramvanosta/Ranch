import { TriangleAlertIcon } from "lucide-react"
import { useEffect, useState } from "react"
import { useCapabilities } from "@/capabilities"
import { displayChord } from "@/commands/chords"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import { usePauseScheduler, useResumeScheduler } from "@/data/schedulers/mutations"
import { useCronJobs } from "@/data/schedulers/queries"
import { StaleNotice } from "@/features/freshness"
import type { CronJob, QueueId } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"
import { formatEveryMs } from "./formatEvery"
import { SchedulerSheet } from "./SchedulerSheet"

/**
 * Table view of a queue's CRON / repeatable schedulers.
 *
 * Columns: Key / Name / Pattern / Next run / TZ / Iterations.
 *
 * Past-due highlighting: when `next_run < now`, the row gets an amber
 * left border + a ⚠ icon with a tooltip explaining the skip. Threshold
 * is strict less-than now (a few seconds of clock skew
 * shouldn't trigger the warning, but anything past should already be
 * the NEXT iteration's score).
 *
 * No virtualization: realistic scheduler counts are <100 per queue.
 *
 * **New scheduler** above the table, **Edit** on each row (the
 * sheet also deletes), when the backend writes schedulers, and Pause /
 * Resume when it pauses them. Disabled with the reason on a locked
 * connection.
 */
interface CronJobsViewProps {
  connectionId: string
  queueId: QueueId
  queueName: string
  /** The connection is locked: writing is refused anyway. */
  locked?: boolean
}

const LOCKED_HINT = `This connection is read-only. Unlock it from the toolbar (${displayChord("cmd+shift+u")}) to change schedulers.`

export function CronJobsView({
  connectionId,
  queueId,
  queueName,
  locked = false,
}: CronJobsViewProps) {
  const { data, isLoading, isError, error, refetch, dataUpdatedAt } = useCronJobs({
    connectionId,
    queueId,
  })
  // `undefined` = closed, `null` = new, a CronJob = edit.
  const [sheet, setSheet] = useState<CronJob | null | undefined>(undefined)
  const { can } = useCapabilities(connectionId)
  const writes = can("schedules.write")
  const pauses = can("schedules.pause")

  // Frozen "now" per fetch; avoids past-due flickering as the user
  // re-renders the row. Updates when the query refetches.
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (data) setNow(Date.now())
  }, [data])

  if (isLoading) {
    return <CronJobsSkeleton />
  }

  // The error box only when there is nothing to show.
  if (isError && !data) {
    return (
      <div
        role="alert"
        className="flex flex-col gap-2 rounded-md border border-(--danger)/40 bg-(--danger)/10 p-3 text-sm text-(--state-failed-text)"
        data-testid="cron-jobs-error"
      >
        <div className="flex items-start gap-2">
          <TriangleAlertIcon className="mt-0.5 h-4 w-4 shrink-0" />
          <div>
            <p className="font-medium">Could not list CRON jobs</p>
            <p className="mt-0.5 text-xs">
              {error instanceof Error ? error.message : "Unknown error"}
            </p>
          </div>
        </div>
        <Button variant="default" size="sm" onClick={() => refetch()}>
          Retry
        </Button>
      </div>
    )
  }

  const sheetElement = (
    <SchedulerSheet
      open={sheet !== undefined}
      onOpenChange={(open) => !open && setSheet(undefined)}
      connectionId={connectionId}
      queueId={queueId}
      queueName={queueName}
      scheduler={sheet ?? null}
    />
  )
  const newButton = writes ? (
    <Button
      variant="primary"
      type="button"
      size="sm"
      onClick={() => setSheet(null)}
      disabled={locked}
      title={locked ? LOCKED_HINT : undefined}
      data-testid="scheduler-new"
    >
      New scheduler
    </Button>
  ) : null

  if (!data || data.length === 0) {
    return (
      <div
        className="flex flex-col items-center gap-3 rounded-md border border-dashed border-(--line) bg-(--surf)/20 p-6 text-center"
        data-testid="cron-jobs-empty"
      >
        <p className="text-sm font-medium">No scheduler in this queue.</p>
        {writes ? (
          <p className="max-w-prose text-xs text-(--text2)">
            A scheduler adds a job on a cron pattern or at a fixed interval. Create one here, or
            with <code className="font-mono">queue.upsertJobScheduler(id, repeat, template)</code>{" "}
            in your code.
          </p>
        ) : null}
        {newButton}
        {sheetElement}
      </div>
    )
  }

  return (
    <TooltipProvider>
      {isError ? (
        <StaleNotice
          error={error}
          updatedAt={dataUpdatedAt}
          what="the schedulers"
          className="mb-2"
        />
      ) : null}
      <div className="mb-2 flex items-center justify-between gap-2">
        <p className="text-xs text-(--text2)">
          {data.length} {data.length === 1 ? "scheduler" : "schedulers"}
        </p>
        {newButton}
      </div>
      <div
        className="overflow-x-auto rounded-md border border-(--line)/40"
        data-testid="cron-jobs-view"
      >
        <table className="w-full text-sm">
          <thead className="bg-(--surf)/40">
            <tr className="text-left">
              <Th>Key</Th>
              <Th>Name</Th>
              <Th>Pattern</Th>
              <Th>Next run</Th>
              <Th>TZ</Th>
              <Th>Iterations</Th>
              {/* No action declared: no column for them. */}
              {writes || pauses ? <Th>Actions</Th> : null}
            </tr>
          </thead>
          <tbody>
            {data.map((cj) => (
              <CronRow
                key={cj.key}
                cj={cj}
                now={now}
                connectionId={connectionId}
                queueId={queueId}
                locked={locked}
                canWrite={writes}
                canPause={pauses}
                onEdit={() => setSheet(cj)}
              />
            ))}
          </tbody>
        </table>
      </div>
      {sheetElement}
    </TooltipProvider>
  )
}

function CronRow({
  cj,
  now,
  connectionId,
  queueId,
  locked,
  canWrite,
  canPause,
  onEdit,
}: {
  cj: CronJob
  now: number
  connectionId: string
  queueId: QueueId
  locked: boolean
  canWrite: boolean
  canPause: boolean
  onEdit: () => void
}) {
  // Past-due only meaningful for *running* schedulers; paused ones
  // have a stale next-run timestamp by design.
  const pastDue = !cj.paused && cj.nextRun < now
  const nextRunLocal = formatLocalDateTime(cj.nextRun)
  const nextRunIso = new Date(cj.nextRun).toISOString()
  const overdueAge = pastDue ? formatRelativeAge(now - cj.nextRun) : null

  const patternCell =
    cj.pattern !== null && cj.pattern !== undefined ? (
      cj.pattern
    ) : cj.every !== null && cj.every !== undefined ? (
      `every ${formatEveryMs(cj.every)}`
    ) : (
      <span className="text-(--text2) italic">unknown</span>
    )

  return (
    <tr
      className={cn(
        "border-t border-(--line)/40 hover:bg-(--surf)/30",
        pastDue && "border-l-2 border-l-amber-500",
      )}
      data-testid={`cron-row-${cj.key}`}
      data-past-due={pastDue ? "true" : "false"}
      data-paused={cj.paused ? "true" : "false"}
    >
      <Td mono>{cj.key}</Td>
      <Td>
        <span className="inline-flex items-center gap-2">
          {cj.name}
          {cj.paused ? (
            <span
              className="rounded-md bg-(--state-delayed)/15 px-1.5 py-0.5 text-2xs font-medium uppercase tracking-wide text-(--state-delayed-text)"
              data-testid={`cron-row-paused-badge-${cj.key}`}
            >
              Paused
            </span>
          ) : null}
        </span>
      </Td>
      <Td mono>{patternCell}</Td>
      <Td mono>
        {cj.paused ? (
          <span className="text-(--text2) italic">Paused</span>
        ) : (
          <span className="inline-flex items-center gap-1.5">
            {pastDue ? (
              <Tooltip>
                <TooltipTrigger
                  render={
                    <span
                      title={`Past-due: last expected run was ${overdueAge} ago`}
                      className="text-(--state-delayed-text)"
                      data-testid={`cron-row-past-due-icon-${cj.key}`}
                    >
                      <TriangleAlertIcon className="size-3.5" aria-hidden="true" />
                    </span>
                  }
                />
                <TooltipContent>
                  Last expected run was {overdueAge} ago. The queue may be paused or the worker
                  idle.
                </TooltipContent>
              </Tooltip>
            ) : null}
            <span title={nextRunIso}>{nextRunLocal}</span>
          </span>
        )}
      </Td>
      <Td>{cj.tz ?? <span className="text-(--text2)">-</span>}</Td>
      <Td mono>{cj.iterationCount}</Td>
      {canWrite || canPause ? (
        <Td>
          <span className="inline-flex gap-1.5">
            {canWrite ? (
              <Button
                type="button"
                size="sm"
                variant="default"
                onClick={onEdit}
                disabled={locked}
                title={locked ? LOCKED_HINT : undefined}
                data-testid={`cron-row-edit-${cj.key}`}
              >
                Edit
              </Button>
            ) : null}
            {canPause ? (
              <CronRowActions
                connectionId={connectionId}
                queueId={queueId}
                schedulerKey={cj.key}
                paused={cj.paused}
              />
            ) : null}
          </span>
        </Td>
      ) : null}
    </tr>
  )
}

/**
 * Pause/Resume per row in the CronJobsView. The button
 * label flips on `paused`; the busy state disables the button while
 * the mutation is in flight to prevent double-clicks.
 */
function CronRowActions({
  connectionId,
  queueId,
  schedulerKey,
  paused,
}: {
  connectionId: string
  queueId: QueueId
  schedulerKey: string
  paused: boolean
}) {
  const pauseMut = usePauseScheduler()
  const resumeMut = useResumeScheduler()
  const busy = pauseMut.isPending || resumeMut.isPending
  const onClick = () => {
    if (busy) return
    const input = { connectionId, queueId, schedulerKey }
    if (paused) {
      resumeMut.mutate(input)
    } else {
      pauseMut.mutate(input)
    }
  }
  return (
    <Button
      type="button"
      size="sm"
      variant="default"
      disabled={busy}
      onClick={onClick}
      data-testid={`cron-row-toggle-${schedulerKey}`}
    >
      {paused ? "Resume" : "Pause"}
    </Button>
  )
}

function Th({ children }: { children: React.ReactNode }) {
  return (
    <th className="px-3 py-2 text-xs font-medium text-(--text2) uppercase tracking-wide">
      {children}
    </th>
  )
}

function Td({ children, mono }: { children: React.ReactNode; mono?: boolean }) {
  return <td className={cn("px-3 py-2", mono && "font-mono text-xs tabular-nums")}>{children}</td>
}

function CronJobsSkeleton() {
  return (
    <div
      role="status"
      aria-label="Loading CRON jobs"
      className="flex flex-col gap-1 rounded-md border border-(--line)/40 p-2"
    >
      {[0, 1, 2].map((i) => (
        <div key={i} className="flex items-center gap-3 px-1 py-2">
          <Skeleton className="h-4 w-24" />
          <Skeleton className="h-4 flex-1" />
          <Skeleton className="h-4 w-32" />
          <Skeleton className="h-4 w-12" />
        </div>
      ))}
    </div>
  )
}

function formatLocalDateTime(ms: number): string {
  const d = new Date(ms)
  // Local date + 24h time, matches the Intl.DateTimeFormat default
  // shape but stays deterministic for tests.
  const pad = (n: number) => String(n).padStart(2, "0")
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

function formatRelativeAge(deltaMs: number): string {
  if (deltaMs < 0) return "0s"
  const s = Math.floor(deltaMs / 1000)
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m`
  const h = Math.floor(m / 60)
  if (h < 24) return `${h}h`
  const d = Math.floor(h / 24)
  return `${d}d`
}
