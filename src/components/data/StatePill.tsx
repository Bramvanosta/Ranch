import { describeState, type JobStateInfo, type StateTone } from "@/domain/jobStates"
import type { JobState } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

// The five state tokens; neutral for a state the backend does not describe.
const TONE: Record<StateTone, string> = {
  pending: "bg-(--state-waiting)/15 text-(--state-waiting-text)",
  running: "bg-(--state-active)/15 text-(--state-active-text)",
  success: "bg-(--state-completed)/15 text-(--state-completed-text)",
  danger: "bg-(--state-failed)/15 text-(--state-failed-text)",
  warning: "bg-(--state-delayed)/15 text-(--state-delayed-text)",
  neutral: "bg-(--surf2) text-(--text2)",
}

/**
 * A job state as a small pill: one look for lists, flows and the inspector,
 * with the label and tone `states` (the connection's, `useJobStates`) give it.
 */
export function StatePill({
  state,
  states,
  className,
  ...props
}: {
  state: JobState
  states: ReadonlyArray<JobStateInfo>
  className?: string
} & React.ComponentProps<"span">) {
  const { label, tone } = describeState(state, states)
  return (
    <span
      data-slot="state-pill"
      data-state={state}
      title={`State: ${label.toLowerCase()}`}
      className={cn(
        "inline-flex shrink-0 items-center rounded-tag px-1.5 py-px text-2xs font-semibold tracking-wide whitespace-nowrap uppercase",
        TONE[tone],
        className,
      )}
      {...props}
    >
      {label.toLowerCase()}
    </span>
  )
}
