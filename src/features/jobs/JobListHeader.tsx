import { ArrowDownIcon, ArrowUpDownIcon, ArrowUpIcon } from "lucide-react"

import type { JobSort, JobSortField } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

/**
 * A job list column header that sorts: ascending → descending →
 * the list's natural order. Without `onSortChange` (the all-queues scope,
 * which cannot sort server-side) it is plain text.
 */
export function nextSort(current: JobSort | null | undefined, field: JobSortField): JobSort | null {
  if (!current || current.field !== field) return { field, descending: false }
  if (!current.descending) return { field, descending: true }
  return null
}

export function SortableHeader({
  label,
  field,
  sort,
  onSortChange,
}: {
  label: string
  field: JobSortField
  sort?: JobSort | null
  onSortChange?: (next: JobSort | null) => void
}) {
  if (!onSortChange) return <span>{label}</span>
  const active = sort?.field === field
  const direction = active ? (sort?.descending ? "descending" : "ascending") : "none"
  return (
    <button
      type="button"
      onClick={() => onSortChange(nextSort(sort, field))}
      data-testid={`job-list-sort-${field}`}
      title={
        active
          ? `Sorted by ${label.toLowerCase()}, ${direction}. Click to change`
          : `Sort every job of this state by ${label.toLowerCase()}`
      }
      className={cn(
        "inline-flex items-center gap-1 rounded-sm uppercase outline-none hover:text-(--text) focus-visible:ring-2 focus-visible:ring-(--accent)/50",
        active && "text-(--text)",
      )}
    >
      {label}
      {/* Icons, not the ↕ ▲ ▼ characters: macOS draws those as emoji. */}
      {active ? (
        sort?.descending ? (
          <ArrowDownIcon className="size-3" aria-hidden="true" />
        ) : (
          <ArrowUpIcon className="size-3" aria-hidden="true" />
        )
      ) : (
        <ArrowUpDownIcon className="size-3 opacity-50" aria-hidden="true" />
      )}
    </button>
  )
}
