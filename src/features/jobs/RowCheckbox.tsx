import { displayChord } from "@/commands/chords"
import type { Job } from "@/lib/tauri/bindings"
import { jobSelectionKey, useJobSelectionStore } from "@/stores/useJobSelectionStore"

/**
 * A job row's selection box: a click toggles the row, ⇧-click
 * extends the selection from the last row clicked. Its clicks never open
 * the job (they stop at the box).
 */
export function RowCheckbox({
  job,
  index,
  onClickCheckbox,
  onShiftClickCheckbox,
}: {
  job: Job
  index?: number
  onClickCheckbox?: (index: number) => void
  onShiftClickCheckbox?: (index: number) => void
}) {
  const key = jobSelectionKey({ jobRef: job.jobRef, state: job.state })
  const selected = useJobSelectionStore((s) => s.selection.has(key))
  const toggle = useJobSelectionStore((s) => s.toggle)
  return (
    <input
      type="checkbox"
      checked={selected}
      onChange={() => {
        // The click handler below decides (it sees ⇧).
      }}
      aria-label={`Select job ${job.jobRef.id}`}
      data-testid="job-list-row-checkbox"
      className="size-4 shrink-0 rounded border border-(--line)"
      onClick={(e) => {
        e.stopPropagation()
        if (e.shiftKey && index !== undefined && onShiftClickCheckbox) {
          onShiftClickCheckbox(index)
        } else {
          if (index !== undefined) onClickCheckbox?.(index)
          toggle({ jobRef: job.jobRef, state: job.state })
        }
      }}
    />
  )
}

/** The page's select-all box: how much of the page is selected, and the toggle (⌘A selects too). */
export function SelectPageCheckbox({
  state,
  onToggle,
}: {
  state: "none" | "some" | "all"
  onToggle: () => void
}) {
  const label =
    state === "all" ? "Deselect every job on this page" : "Select every job on this page"
  return (
    <input
      type="checkbox"
      checked={state === "all"}
      ref={(el) => {
        if (el) el.indeterminate = state === "some"
      }}
      onChange={onToggle}
      aria-label={label}
      title={state === "all" ? label : `${label} (${displayChord("cmd+a")})`}
      data-testid="job-list-select-page"
      className="size-4 shrink-0 rounded border border-(--line)"
    />
  )
}
