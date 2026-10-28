import { type LucideIcon, SearchIcon, TriangleAlertIcon, XIcon } from "lucide-react"
import { type ReactNode, useEffect, useRef } from "react"
import { toast } from "sonner"

import { useCommandHandler } from "@/commands/handlers"
import { Button } from "@/components/ui/button"
import { useDismissedHint } from "@/hooks/useDismissedHint"
import { cn } from "@/lib/utils"

/**
 * Filter input + scope toggle + match counter for the
 * job list. Sits between `JobStateChips` and `JobList` on the queue
 * detail route's Jobs view.
 *
 * In queue scope the two segments are applied **server-side
 * across the whole state** (`list_jobs` `search` / `reason`), so the counter
 * reads "matching of all jobs in the state" and `scanNote` reports a capped
 * scan. Cross-queue scope still filters the loaded rows (`filterJobs`).
 *
 * Anatomy (left → right):
 *   1. Two-segment filter input: a single rounded container with two
 *      `<input>` zones (name or id + error), each behind its icon,
 *      separated by a thin divider. Each segment shows a clear button when
 *      non-empty. The error segment only shows where a list can hold errors
 *      (`showErrorFilter`), or while it still filters something.
 *   2. Scope: two buttons, "This queue" / "All queues": the choice and the
 *      alternative are both on screen (it was one button whose label was its
 *      current value). `cmd-shift-S` (mac) / `ctrl-shift-S` (Linux/Win)
 *      flips it.
 *   3. Match counter: `{filtered} of {total} jobs` when a filter is
 *      active, else just `{total} jobs`. `font-tabular` so the digits
 *      don't jitter as the user types.
 *
 * Below the row, when the filter excludes every job (filteredCount === 0
 * AND totalCount > 0), a small inline helper renders with two CTAs:
 *   - `[Clear filter]`: empties both segments.
 *   - `[Expand to all queues]`: flips scope to cross-queue (shown only
 *     when the current scope is `"queue"`).
 *
 * One-time hint: the first time the user activates cross-queue scope on
 * THIS machine ever, a sonner toast appears with a discoverability tip.
 * Dismissal is persisted in `kv_settings` via `useDismissedHint` so it
 * never reappears.
 */
export type JobFilterScope = "queue" | "cross"

export interface JobFilterValue {
  name: string
  error: string
}

interface JobFilterProps {
  filter: JobFilterValue
  onFilterChange: (next: JobFilterValue) => void
  scope: JobFilterScope
  onScopeChange: (next: JobFilterScope) => void
  totalCount: number
  filteredCount: number
  /** One line under the inputs (e.g. the search hit its scan cap). */
  scanNote?: string | null
  /** A server-side search / sort is running: the counter says so. */
  searching?: boolean
  /** The list on screen can hold errors (failed, delayed): offer the error filter. */
  showErrorFilter?: boolean
  /** Before the inputs: the Failed tab's List / By cause switch. */
  leading?: ReactNode
}

export function JobFilter({
  filter,
  onFilterChange,
  scope,
  onScopeChange,
  totalCount,
  filteredCount,
  scanNote = null,
  searching = false,
  showErrorFilter = true,
  leading,
}: JobFilterProps) {
  const { dismissed, dismiss } = useDismissedHint("cross-queue-scope")

  // Edit › Find (⌘F) puts the cursor in the name filter. The
  // scope toggle lost its ⇧⌘S chord: that is Save As on every platform.
  const rootRef = useRef<HTMLDivElement>(null)
  useCommandHandler("edit.find", () => {
    const input = rootRef.current?.querySelector<HTMLInputElement>("[data-testid=job-filter-name]")
    input?.focus()
    input?.select()
  })

  // One-time hint: fire when scope transitions to "cross" the very first
  // time the user does so on this machine. `dismissed === undefined`
  // means the kv read is still in flight; wait before firing to avoid
  // a false-positive on slow disks. `dismiss` is stable (useCallback
  // inside useDismissedHint) so including it in deps doesn't refire.
  useEffect(() => {
    if (scope !== "cross") return
    if (dismissed !== false) return
    toast("Cross-queue scope filters across every queue in this connection.", {
      description: "Click “This queue” to come back. Click × to dismiss this tip.",
      duration: 8_000,
      action: {
        label: "Got it",
        onClick: () => dismiss(),
      },
    })
    dismiss()
  }, [scope, dismissed, dismiss])

  const hasFilter = filter.name.trim() !== "" || filter.error.trim() !== ""
  const showEmptyHelper = hasFilter && filteredCount === 0 && totalCount > 0

  return (
    <div ref={rootRef} className="flex flex-col gap-2" data-testid="job-filter">
      <div className="flex items-center gap-2">
        {leading}
        {/* Two-segment input. Visually one rounded container with a
            vertical divider between the name and error inputs. */}
        <div
          className={cn(
            "flex h-9 flex-1 items-center gap-2 rounded-md border border-(--line) bg-transparent px-2 text-sm shadow-xs",
            "focus-within:border-(--accent) focus-within:ring-[3px] focus-within:ring-(--accent)/50",
            "dark:bg-(--field)/30",
          )}
          data-testid="job-filter-input-container"
        >
          <FilterSegment
            icon={SearchIcon}
            value={filter.name}
            placeholder="Search by job name or id"
            ariaLabel="Filter by job name or id"
            onChange={(v) => onFilterChange({ ...filter, name: v })}
            testId="job-filter-name"
          />
          {showErrorFilter || filter.error !== "" ? (
            <>
              <span aria-hidden="true" className="h-5 w-px bg-(--line)" />
              <FilterSegment
                icon={TriangleAlertIcon}
                value={filter.error}
                placeholder="Search in error messages"
                ariaLabel="Filter by error message"
                onChange={(v) => onFilterChange({ ...filter, error: v })}
                testId="job-filter-error"
              />
            </>
          ) : null}
        </div>

        {/* Scope: both choices on screen */}
        {/* biome-ignore lint/a11y/useSemanticElements: a two-button toggle group, not a form fieldset */}
        <div
          role="group"
          aria-label="Search scope"
          data-testid="job-filter-scope"
          data-scope={scope}
          className="inline-flex h-9 shrink-0 items-center rounded-md border border-(--line) p-0.5 text-xs font-medium"
        >
          {(
            [
              ["queue", "This queue"],
              ["cross", "All queues"],
            ] as const
          ).map(([value, label]) => (
            <button
              key={value}
              type="button"
              aria-pressed={scope === value}
              onClick={() => onScopeChange(value)}
              data-testid={`job-filter-scope-${value}`}
              className={cn(
                "h-full rounded-[5px] px-2.5 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)",
                scope === value
                  ? "bg-(--surf) text-(--text)"
                  : "text-(--text2) hover:text-(--text)",
              )}
            >
              {label}
            </button>
          ))}
        </div>

        {/* Match counter */}
        <span
          className="shrink-0 font-tabular text-xs text-(--text2)"
          data-testid="job-filter-counter"
          aria-live="polite"
        >
          {searching
            ? "Searching…"
            : hasFilter
              ? `${filteredCount.toLocaleString()} of ${totalCount.toLocaleString()} jobs`
              : `${totalCount.toLocaleString()} jobs`}
        </span>
      </div>

      {scanNote ? (
        <p className="text-2xs italic text-(--text2)" data-testid="job-filter-scan-note">
          {scanNote}
        </p>
      ) : null}

      {showEmptyHelper ? (
        <div
          className="flex flex-wrap items-center gap-2 rounded-md border border-dashed border-(--line) bg-(--surf)/20 p-3 text-sm"
          data-testid="job-filter-empty-helper"
        >
          <span className="text-(--text2)">
            No jobs match the filter. Try clearing the filter or expanding scope.
          </span>
          <div className="ml-auto flex items-center gap-1.5">
            <Button
              variant="default"
              size="sm"
              onClick={() => onFilterChange({ name: "", error: "" })}
              data-testid="job-filter-clear"
            >
              Clear filter
            </Button>
            {scope === "queue" ? (
              <Button
                variant="default"
                size="sm"
                onClick={() => onScopeChange("cross")}
                data-testid="job-filter-expand"
              >
                Expand to all queues
              </Button>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  )
}

function FilterSegment({
  icon: Icon,
  value,
  placeholder,
  ariaLabel,
  onChange,
  testId,
}: {
  icon: LucideIcon
  value: string
  placeholder: string
  ariaLabel: string
  onChange: (v: string) => void
  testId: string
}) {
  return (
    <div className="flex min-w-0 flex-1 items-center gap-2">
      <Icon className="size-3.5 shrink-0 text-(--text2)" aria-hidden="true" />
      <input
        type="text"
        value={value}
        placeholder={placeholder}
        aria-label={ariaLabel}
        onChange={(e) => onChange(e.target.value)}
        className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-(--text2)"
        data-testid={testId}
      />
      {value !== "" ? (
        <button
          type="button"
          onClick={() => onChange("")}
          aria-label={`Clear: ${ariaLabel}`}
          className="ml-1 rounded-sm text-(--text2) hover:text-(--text) focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)"
          data-testid={`${testId}-clear`}
        >
          <XIcon className="size-3.5" aria-hidden="true" />
        </button>
      ) : null}
    </div>
  )
}
