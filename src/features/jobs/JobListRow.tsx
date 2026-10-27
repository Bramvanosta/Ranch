/**
 * One row of a job list: its cells come from the list's
 * columns (`jobColumns` — what matters for that state), laid out on the
 * table's grid (`DataTable`). The row itself is what is common to every
 * state: selection, opening the job, the right-click menu, the keyboard
 * mark, and what a screen reader hears.
 *
 * One line (34 px) for the usual job; a long error or job id wraps and the
 * row grows — nothing in a row is cut.
 *
 *   - a plain click (or Enter) opens the job: beside the list in the
 *     screen's inspector (`onOpen`), else on its own page;
 *   - ⌘-click adds or removes the row from the selection, ⇧-click extends
 *     it from the last row clicked;
 *   - right-click acts on the selection when the row is part of it, else on
 *     the row.
 *
 * The keyboard mark is a bar on the left edge, and only once the keyboard
 * has been used on the list.
 */
import { useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useMemo } from "react"

import { DATA_TABLE_GRID, useVisibleColumnIds } from "@/components/data/DataTable"
import { STATE_LABELS } from "@/components/data/StatePill"
import { type MenuAction, separator, TargetMenu } from "@/components/menus/TargetMenu"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import type { Job } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"
import { jobRoute, openRouteOrSay } from "@/lib/windows/openRoute"
import { jobSelectionKey, useJobSelectionStore } from "@/stores/useJobSelectionStore"

import { formatAge } from "./formatAge"
import { firstLine, type JobColumn, jobColumns } from "./jobColumns"
import { useFocusedJobStore } from "./useFocusedJobStore"
import { usePromoteJob, useRetryJob } from "./useJobActions"

/** A one-line row; rows with wrapped text are measured taller. */
export const JOB_ROW_HEIGHT_PX = 34

interface JobListRowProps {
  job: Job
  /** Frozen "now" passed in by the list so all rows share one snapshot — avoids
   *  jitter where row 1's age says `2m` while row 50's says `2m1s`. */
  now: number
  /** Connection ID used to build the drill-down route. */
  connectionId: string
  /** The list's columns; a row on its own uses its state's. */
  columns?: JobColumn[]
  /** Row index in the rendered list (for shift-click range). */
  index?: number
  /** shift-click handler. */
  onShiftClickCheckbox?: (index: number) => void
  /** Plain click handler. */
  onClickCheckbox?: (index: number) => void
  /**
   * What a plain click (or Enter) does: open the job in the
   * screen's inspector. Without it the row opens the job's own page.
   */
  onOpen?: (job: Job) => void
}

export function JobListRow({
  job,
  now,
  connectionId,
  columns: listColumns,
  index,
  onShiftClickCheckbox,
  onClickCheckbox,
  onOpen,
}: JobListRowProps) {
  const navigate = useNavigate()
  const copy = useCopyToClipboard()
  const columns = useMemo(
    () => listColumns ?? jobColumns({ state: job.state }),
    [listColumns, job.state],
  )
  const visible = useVisibleColumnIds()
  const shown = visible === null ? columns : columns.filter((c) => visible.has(c.id))

  const ageStr = formatAge(now - job.timestamp)
  const stateLabel = STATE_LABELS[job.state] ?? "unknown"
  const hasFailedReason = job.failedReason !== null && job.failedReason.length > 0

  // Selection state for this row.
  const entryKey = jobSelectionKey({ jobRef: job.jobRef, state: job.state })
  const isSelected = useJobSelectionStore((s) => s.selection.has(entryKey))
  const toggle = useJobSelectionStore((s) => s.toggle)
  // Keyboard focus highlight.
  const isFocused = useFocusedJobStore((s) => s.focusedIndex === index)

  const ariaLabel =
    `Job ${job.jobRef.id}, ${job.name}, ${stateLabel}, ${ageStr} ago` +
    (job.failedReason ? `, error: ${firstLine(job.failedReason).slice(0, 80)}` : "")

  const drillTo = `/connections/${connectionId}/queues/${encodeURIComponent(job.jobRef.queue.name)}/${encodeURIComponent(job.jobRef.id)}`

  const queryClient = useQueryClient()
  const openJob = () => {
    // The row already holds the job: the inspector opens on it
    // at once, marked old enough that a fresh read starts right away.
    const jobKey = [
      "job",
      connectionId,
      job.jobRef.queue.prefix,
      job.jobRef.queue.name,
      job.jobRef.id,
    ]
    if (queryClient.getQueryData(jobKey) === undefined) {
      queryClient.setQueryData(jobKey, job, { updatedAt: Date.now() - 60_000 })
    }
    if (onOpen) {
      onOpen(job)
      return
    }
    void navigate({
      to: "/connections/$connectionId/queues/$queueName/$jobId",
      params: {
        connectionId,
        queueName: job.jobRef.queue.name,
        jobId: job.jobRef.id,
      },
    })
  }
  // ⌘-click adds or removes the row from the
  // selection, ⇧-click extends it from the last row clicked, a plain click opens.
  const handleRowClick = (e?: React.MouseEvent) => {
    if (e && (e.metaKey || e.ctrlKey)) {
      if (index !== undefined) onClickCheckbox?.(index)
      toggle({ jobRef: job.jobRef, state: job.state })
      return
    }
    if (e?.shiftKey && index !== undefined && onShiftClickCheckbox) {
      onShiftClickCheckbox(index)
      return
    }
    openJob()
  }

  // right-click: on a selected row with others selected, the
  // menu acts on the whole selection and says so in its first line.
  const retry = useRetryJob()
  const promote = usePromoteJob()
  const menuActions = (): MenuAction[] => {
    const selection = useJobSelectionStore.getState()
    const size = selection.selection.size
    const copies: MenuAction[] = [
      {
        label: "Copy Job ID",
        run: () => void copy(job.jobRef.id, "job ID"),
        testId: "job-list-row-copy-id",
      },
      {
        label: "Copy Payload",
        run: () => void copy(job.data, "payload"),
        testId: "job-list-row-copy-payload",
      },
      ...(hasFailedReason
        ? [
            {
              label: "Copy Stack",
              run: () =>
                void copy(`${job.failedReason}\n\n${job.stacktrace.join("\n")}`, "stack trace"),
              testId: "job-list-row-copy-stack",
            },
          ]
        : []),
    ]
    if (isSelected && size > 1) {
      const jobs = `${size} Jobs`
      return [
        {
          label: `Retry ${jobs}`,
          shortcut: "cmd+shift+r",
          run: () => selection.requestAction("retry"),
        },
        {
          label: `Promote ${jobs}`,
          shortcut: "alt+cmd+p",
          run: () => selection.requestAction("promote"),
        },
        {
          label: `Remove ${jobs}…`,
          shortcut: "cmd+backspace",
          danger: true,
          run: () => selection.requestAction("remove"),
        },
        separator,
        ...copies,
      ]
    }
    const input = { connectionId, queueRef: job.jobRef.queue, jobId: job.jobRef.id }
    return [
      { label: "Open", run: openJob },
      {
        label: "Open in New Window",
        shortcut: isFocused ? "alt+enter" : undefined,
        run: () =>
          openRouteOrSay(jobRoute(connectionId, job.jobRef.queue.name, job.jobRef.id), {
            tab: false,
          }),
      },
      separator,
      { label: "Retry", disabled: job.state !== "failed", run: () => retry.mutate(input) },
      { label: "Promote", disabled: job.state !== "delayed", run: () => promote.mutate(input) },
      {
        label: "Remove…",
        danger: true,
        // Same path as ⌘⌫ on a row: select it, the selection bar confirms.
        run: () => {
          if (!isSelected) selection.add({ jobRef: job.jobRef, state: job.state })
          selection.requestAction("remove")
        },
      },
      separator,
      ...copies,
    ]
  }

  const ctx = { now, connectionId, index, onClickCheckbox, onShiftClickCheckbox }

  return (
    <TargetMenu
      actions={menuActions}
      render={
        // biome-ignore lint/a11y/useSemanticElements: ARIA-grid row for the virtualized list (parent is role="table"); using <tr> doesn't compose with @tanstack/react-virtual.
        <div
          role="row"
          tabIndex={0}
          data-testid="job-list-row"
          data-selected={isSelected ? "true" : undefined}
          data-focused={isFocused ? "true" : undefined}
          data-href={drillTo}
          aria-label={ariaLabel}
          onClick={handleRowClick}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault()
              handleRowClick()
            }
          }}
          className={cn(
            DATA_TABLE_GRID,
            "group min-h-[34px] cursor-pointer border-b border-(--line) px-3 py-1.5 text-sm hover:bg-(--surf2) focus-within:bg-(--surf2) focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-(--accent)/60",
            isSelected && "bg-(--sel)",
            isFocused && "shadow-[inset_2px_0_0_var(--accent)]",
          )}
        >
          {shown.map((c) => (
            // biome-ignore lint/a11y/noStaticElementInteractions: event-isolation shield — the checkbox and the action buttons own their clicks; the row's click must not also open the job.
            <div
              key={c.id}
              className={cn("flex min-w-0 items-center", c.align === "end" && "justify-end")}
              // The checkbox and the actions are clicks of their own, not "open the job".
              onClick={
                c.id === "select" || c.id === "actions" ? (e) => e.stopPropagation() : undefined
              }
              onKeyDown={
                c.id === "select" || c.id === "actions" ? (e) => e.stopPropagation() : undefined
              }
            >
              {c.cell(job, ctx)}
            </div>
          ))}
        </div>
      }
    />
  )
}
