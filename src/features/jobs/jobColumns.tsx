import type { ReactNode } from "react"

import type { DataTableColumn } from "@/components/data/DataTable"
import { formatDurationMs } from "@/features/metrics/format"
import type { Job, JobSort, JobState } from "@/lib/tauri/bindings"
import { ActiveProgressCell, ActiveRunningCell } from "./ActiveJobCells"
import { formatAge } from "./formatAge"
import { JobActions } from "./JobActions"
import { SortableHeader } from "./JobListHeader"
import { jobDurationMs } from "./jobDuration"
import { attemptsLabel, summarizeError } from "./jobPreview"
import { RowCheckbox, SelectPageCheckbox } from "./RowCheckbox"

/** What a cell may need beyond its job. */
export interface CellContext {
  now: number
  connectionId: string
  index?: number
  onClickCheckbox?: (index: number) => void
  onShiftClickCheckbox?: (index: number) => void
}

export interface JobColumn extends DataTableColumn {
  cell: (job: Job, ctx: CellContext) => ReactNode
}

const MONO = "font-mono text-xs tabular-nums text-(--text2)"

/** The first line of a failure, in full: the rest is in the inspector. */
export function firstLine(text: string): string {
  return text.split("\n")[0]?.trim() ?? ""
}

function opt<T>(opts: string, key: string): T | undefined {
  try {
    return (JSON.parse(opts) as Record<string, T>)[key]
  } catch {
    return undefined
  }
}

const ago = (t: number | null, now: number) => (t === null ? "—" : formatAge(now - t))

/**
 * The columns of a job list, per state: what
 * matters for that state, nothing that has to be cut to fit. Rows are one
 * line (34 px); a long error or job id wraps and the row grows.
 *
 *   - Failed: job, ID, last error, tries, failed, ran
 *   - Waiting (and Prioritized, Paused): job, ID, added, priority, delay
 *   - Active: job, ID, progress (and its note), running (against the usual
 *     time of its type), worker, attempt
 *   - Delayed: job, ID, runs in, last error (a retry), tries, added
 *   - Completed: job, ID, finished, took, tries
 *
 * The all-queues scope adds the queue after the ID. `minWidth` drops the
 * least telling columns from a narrow table instead of squeezing them.
 */
export function jobColumns({
  state,
  showQueue = false,
  sort = null,
  onSortChange,
  pageSelection,
  onTogglePage,
}: {
  state: JobState
  showQueue?: boolean
  sort?: JobSort | null
  onSortChange?: (next: JobSort | null) => void
  pageSelection?: "none" | "some" | "all"
  onTogglePage?: () => void
}): JobColumn[] {
  const sortable = (label: string, field: "name" | "queuedAt" | "duration") => (
    <SortableHeader label={label} field={field} sort={sort} onSortChange={onSortChange} />
  )
  // Every sortable column says how it sorts (`aria-sort` on its header cell).
  const sortedOn = (field: "name" | "queuedAt" | "duration") =>
    onSortChange
      ? sort?.field === field
        ? sort.descending
          ? ("descending" as const)
          : ("ascending" as const)
        : ("none" as const)
      : undefined

  const select: JobColumn = {
    id: "select",
    track: "1rem",
    header:
      pageSelection && onTogglePage ? (
        <SelectPageCheckbox state={pageSelection} onToggle={onTogglePage} />
      ) : null,
    cell: (job, ctx) => (
      <RowCheckbox
        job={job}
        index={ctx.index}
        onClickCheckbox={ctx.onClickCheckbox}
        onShiftClickCheckbox={ctx.onShiftClickCheckbox}
      />
    ),
  }
  const name: JobColumn = {
    id: "job",
    track: "minmax(9rem, 1.2fr)",
    header: sortable("Job", "name"),
    ariaSort: sortedOn("name"),
    cell: (job) => (
      <span className="min-w-0 font-medium break-words" data-testid="job-list-row-name">
        {job.name || "(unnamed)"}
      </span>
    ),
  }
  const id: JobColumn = {
    id: "id",
    track: "minmax(5rem, 1fr)",
    header: "ID",
    cell: (job) => (
      <span className={`${MONO} min-w-0 break-all`} data-testid="job-list-row-id">
        #{job.jobRef.id}
      </span>
    ),
  }
  const queue: JobColumn[] = showQueue
    ? [
        {
          id: "queue",
          track: "minmax(5rem, 0.8fr)",
          minWidth: 640,
          header: "Queue",
          cell: (job) => (
            <span className={`${MONO} min-w-0 break-all`} data-testid="job-list-row-queue-cell">
              {job.jobRef.queue.name}
            </span>
          ),
        },
      ]
    : []
  const error: JobColumn = {
    id: "error",
    track: "minmax(12rem, 3fr)",
    header: "Last error",
    cell: (job) =>
      job.failedReason ? (
        <span
          className="min-w-0 font-mono text-xs break-words text-(--state-failed-text)"
          data-testid="job-list-row-error-cell"
        >
          {summarizeError(job.failedReason)}
        </span>
      ) : (
        <span className={MONO}>—</span>
      ),
  }
  const tries: JobColumn = {
    id: "tries",
    track: "3rem",
    minWidth: 560,
    align: "end",
    header: "Tries",
    cell: (job) => (
      <span className={MONO} title="Attempts made / allowed" data-testid="job-list-row-attempts">
        {attemptsLabel(job.attemptsMade, job.opts)}
      </span>
    ),
  }
  const added: JobColumn = {
    id: "added",
    track: "4rem",
    align: "end",
    header: sortable("Added", "queuedAt"),
    ariaSort: sortedOn("queuedAt"),
    cell: (job, ctx) => (
      <span
        className={MONO}
        title={`Added ${new Date(job.timestamp).toLocaleString()}`}
        data-testid="job-list-row-age"
      >
        {formatAge(ctx.now - job.timestamp)}
      </span>
    ),
  }
  const duration = (label: string): JobColumn => ({
    id: "duration",
    track: "4.5rem",
    align: "end",
    header: sortable(label, "duration"),
    ariaSort: sortedOn("duration"),
    cell: (job, ctx) => {
      const ms = jobDurationMs(job, ctx.now)
      return (
        <span
          className={MONO}
          title={ms === null ? "Not yet started" : `${ms} ms`}
          data-testid="job-list-row-duration"
        >
          {ms === null ? "—" : formatDurationMs(ms)}
        </span>
      )
    },
  })
  const at = (colId: string, label: string, pick: (job: Job) => number | null): JobColumn => ({
    id: colId,
    track: "4rem",
    align: "end",
    header: label,
    cell: (job, ctx) => {
      const t = pick(job)
      return (
        <span
          className={MONO}
          title={t === null ? undefined : new Date(t).toLocaleString()}
          data-testid="job-list-row-milestone"
        >
          {ago(t, ctx.now)}
        </span>
      )
    },
  })
  const actions: JobColumn = {
    id: "actions",
    track: "2rem",
    align: "end",
    header: null,
    cell: (job, ctx) =>
      job.state === "failed" || job.state === "delayed" ? (
        <JobActions connectionId={ctx.connectionId} queueRef={job.jobRef.queue} job={job} compact />
      ) : null,
  }

  switch (state) {
    case "failed":
      return [
        select,
        name,
        id,
        ...queue,
        error,
        tries,
        at("failedAt", "Failed", (j) => j.finishedOn),
        duration("Ran"),
        actions,
      ]
    case "active":
      // How far, how long against the usual, who.
      return [
        select,
        name,
        id,
        ...queue,
        {
          id: "progress",
          track: "minmax(9rem, 1fr)",
          header: "Progress",
          cell: (job, ctx) => (
            <ActiveProgressCell job={job} connectionId={ctx.connectionId} now={ctx.now} />
          ),
        },
        {
          id: "running",
          track: "5rem",
          align: "end",
          header: "Running",
          cell: (job, ctx) => (
            <ActiveRunningCell job={job} connectionId={ctx.connectionId} now={ctx.now} />
          ),
        },
        {
          id: "worker",
          track: "minmax(8rem, 0.6fr)",
          minWidth: 640,
          header: "Worker",
          cell: (job) => (
            <span
              className="font-tabular text-xs break-words text-(--text2)"
              data-testid="job-list-row-worker"
            >
              {job.processedBy ?? "—"}
            </span>
          ),
        },
        {
          id: "attempt",
          track: "4rem",
          minWidth: 560,
          align: "end",
          header: "Attempt",
          cell: (job) => (
            <span className={MONO} data-testid="job-list-row-attempt">
              {attemptsLabel(job.attemptsMade + 1, job.opts)}
            </span>
          ),
        },
        actions,
      ]
    case "delayed":
      return [
        select,
        name,
        id,
        ...queue,
        {
          id: "runsIn",
          track: "4.5rem",
          align: "end",
          header: "Runs in",
          cell: (job, ctx) => {
            const due = job.timestamp + (job.delay ?? 0)
            return (
              <span className={MONO} title={new Date(due).toLocaleString()}>
                {due > ctx.now ? formatDurationMs(due - ctx.now) : "now"}
              </span>
            )
          },
        },
        { ...error, track: "minmax(10rem, 2fr)", minWidth: 600 },
        tries,
        { ...added, minWidth: 700 },
        actions,
      ]
    case "completed":
      return [
        select,
        name,
        id,
        ...queue,
        at("finishedAt", "Finished", (j) => j.finishedOn),
        duration("Took"),
        tries,
        actions,
      ]
    default:
      return [
        select,
        name,
        id,
        ...queue,
        added,
        {
          id: "priority",
          track: "4rem",
          align: "end",
          minWidth: 560,
          header: "Priority",
          cell: (job) => <span className={MONO}>{opt<number>(job.opts, "priority") ?? "—"}</span>,
        },
        {
          id: "delay",
          track: "4.5rem",
          align: "end",
          minWidth: 600,
          header: "Delay",
          cell: (job) => {
            const d = opt<number>(job.opts, "delay")
            return <span className={MONO}>{d ? formatDurationMs(d) : "—"}</span>
          },
        },
        actions,
      ]
  }
}
