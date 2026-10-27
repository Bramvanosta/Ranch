import type { QueueCounters as Counters } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

/**
 * Counter pill row.
 *
 * Five pills, color-coded by job state token (`--state-waiting` blue,
 * `--state-active` cyan, `--state-completed` emerald, `--state-failed`
 * red, `--state-delayed` purple). Each count uses the `font-tabular`
 * utility (prevents number jitter on poll refresh).
 *
 * Dropped the refresh pulse dot. It was visual noise per
 * user feedback: counters already animate via their value updates;
 * the dot added nothing actionable. The `isFetching` prop is kept on
 * the signature for backward-compat with existing callers but is no
 * longer rendered. Future cleanup can drop the prop entirely.
 *
 * Each pill is a `<button>` so it's keyboard-focusable and primed to
 * filter the job list. The `onSelectState` callback is
 * optional — when absent, pills are visually identical but
 * inert.
 */
export type QueueState = "waiting" | "active" | "completed" | "failed" | "delayed"

interface QueueCountersProps {
  counters: Counters
  /** Unused since the refresh pulse was dropped. Kept on
   *  the signature to avoid breaking call sites; safe to remove in a
   *  follow-up cleanup. */
  isFetching?: boolean
  onSelectState?: (state: QueueState) => void
}

const PILLS: Array<{
  key: QueueState
  label: string
  className: string
}> = [
  {
    key: "waiting",
    label: "Waiting",
    className: "bg-[var(--state-waiting)]/10 text-[var(--state-waiting)]",
  },
  {
    key: "active",
    label: "Active",
    className: "bg-[var(--state-active)]/10 text-[var(--state-active)]",
  },
  {
    key: "completed",
    label: "Completed",
    className: "bg-[var(--state-completed)]/10 text-[var(--state-completed)]",
  },
  {
    key: "failed",
    label: "Failed",
    className: "bg-[var(--state-failed)]/10 text-(--state-failed-text)",
  },
  {
    key: "delayed",
    label: "Delayed",
    className: "bg-[var(--state-delayed)]/10 text-[var(--state-delayed)]",
  },
]

export function QueueCounters({ counters, onSelectState }: QueueCountersProps) {
  return (
    <div className="flex flex-wrap items-center gap-1.5" data-testid="queue-counters">
      {PILLS.map(({ key, label, className }) => {
        const pillContent = (
          <>
            <span>{label}</span>
            <span className="font-tabular font-medium">{counters[key]}</span>
          </>
        )
        const pillClass = cn(
          "inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-xs",
          "transition-opacity",
          className,
        )
        // When `onSelectState` is provided we render an interactive
        // button. Otherwise (e.g. inside a `<Link>` row) we
        // render a span so the button-inside-anchor HTML invalid-
        // nesting warning doesn't fire.
        return onSelectState ? (
          <button
            type="button"
            key={key}
            onClick={(e) => {
              e.stopPropagation()
              onSelectState(key)
            }}
            aria-label={`${label}: ${counters[key]}`}
            className={cn(
              pillClass,
              "hover:opacity-80",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)",
            )}
          >
            {pillContent}
          </button>
        ) : (
          <span key={key} title={`${label}: ${counters[key]}`} className={pillClass}>
            {pillContent}
          </span>
        )
      })}
    </div>
  )
}
