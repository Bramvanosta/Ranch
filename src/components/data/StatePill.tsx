import type { JobState } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

/** How each BullMQ state reads (lowercase, as BullMQ names them). */
export const STATE_LABELS: Record<JobState, string> = {
  waiting: "waiting",
  active: "active",
  completed: "completed",
  failed: "failed",
  delayed: "delayed",
  prioritized: "prioritized",
  paused: "paused",
  stalled: "stalled",
  waitingChildren: "waiting-children",
  unknown: "unknown",
}

// The five state tokens; the other states borrow the closest one.
const TONE: Record<JobState, string> = {
  waiting: "bg-(--state-waiting)/15 text-(--state-waiting-text)",
  prioritized: "bg-(--state-waiting)/15 text-(--state-waiting-text)",
  active: "bg-(--state-active)/15 text-(--state-active-text)",
  completed: "bg-(--state-completed)/15 text-(--state-completed-text)",
  failed: "bg-(--state-failed)/15 text-(--state-failed-text)",
  stalled: "bg-(--state-failed)/15 text-(--state-failed-text)",
  delayed: "bg-(--state-delayed)/15 text-(--state-delayed-text)",
  waitingChildren: "bg-(--state-delayed)/15 text-(--state-delayed-text)",
  paused: "bg-(--state-delayed)/15 text-(--state-delayed-text)",
  unknown: "bg-(--surf2) text-(--text2)",
}

/** A job state as a small pill: one look for lists, flows and the inspector. */
export function StatePill({
  state,
  className,
  ...props
}: { state: JobState; className?: string } & React.ComponentProps<"span">) {
  return (
    <span
      data-slot="state-pill"
      data-state={state}
      title={`State: ${STATE_LABELS[state] ?? "unknown"}`}
      className={cn(
        "inline-flex shrink-0 items-center rounded-tag px-1.5 py-px text-2xs font-semibold tracking-wide whitespace-nowrap uppercase",
        TONE[state] ?? TONE.unknown,
        className,
      )}
      {...props}
    >
      {STATE_LABELS[state] ?? "unknown"}
    </span>
  )
}
