import { ProgressBar } from "@/components/ui/progress-bar"
import { cn } from "@/lib/utils"

import { parseJobProgress } from "./parseJobProgress"

/**
 * Progress a worker reported for a job. A numeric value renders
 * as a bar + percentage; an object / string as a small text chip. Renders
 * nothing when the job reported no progress.
 */
export function JobProgress({
  progress,
  className,
}: {
  progress: string | null | undefined
  className?: string
}) {
  const parsed = parseJobProgress(progress)
  if (!parsed) return null
  if (parsed.percent === null) {
    return (
      <span
        className={cn(
          "min-w-0 rounded-tag bg-(--surf2) px-1.5 py-0.5 font-tabular text-2xs break-words text-(--text2)",
          className,
        )}
        data-testid="job-progress-text"
      >
        {parsed.label}
      </span>
    )
  }
  return (
    <span className={cn("flex shrink-0", className)} data-testid="job-progress">
      <ProgressBar
        value={parsed.percent}
        label={parsed.label}
        ariaLabel="Job progress"
        tone="active"
        trackClassName="w-14"
      />
    </span>
  )
}
