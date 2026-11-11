import { cn } from "@/lib/utils"

/**
 * A thin progress bar: a 6 px track, the value label
 * outside it, never on top of it. `tone="active"` for work a worker does (job
 * progress), the accent for work the user started (maintenance), `warning`
 * for work that looks stuck. `value={null}` is work of unknown size: a
 * segment sweeps the track (a dimmed full track under reduced motion, which a
 * still segment would misread as a value).
 */
export function ProgressBar({
  value,
  max = 100,
  label,
  ariaLabel,
  tone = "accent",
  className,
  trackClassName,
}: {
  /** `null`: under way, how far unknown. */
  value: number | null
  max?: number
  /** Shown after the bar, in the data face (e.g. "42 %"). */
  label?: string
  ariaLabel: string
  tone?: "accent" | "active" | "warning"
  className?: string
  /** Width of the track; it fills its container by default. */
  trackClassName?: string
}) {
  const tint =
    tone === "active"
      ? "bg-(--state-active)"
      : tone === "warning"
        ? "bg-(--state-delayed)"
        : "bg-(--accent)"
  if (value === null) {
    return (
      <span className={cn("flex min-w-0 items-center gap-2", className)} data-slot="progress-bar">
        <span
          role="progressbar"
          aria-label={ariaLabel}
          className={cn("h-1.5 w-full overflow-hidden rounded-full bg-(--surf2)", trackClassName)}
        >
          <span
            className={cn(
              "block h-full w-1/3 animate-progress-sweep rounded-full motion-reduce:w-full motion-reduce:animate-none motion-reduce:opacity-40",
              tint,
            )}
          />
        </span>
        {label ? (
          <span className="shrink-0 font-tabular text-2xs whitespace-nowrap text-(--text2)">
            {label}
          </span>
        ) : null}
      </span>
    )
  }
  const clamped = Math.min(Math.max(value, 0), max)
  const percent = max > 0 ? (clamped / max) * 100 : 0
  return (
    <span className={cn("flex min-w-0 items-center gap-2", className)} data-slot="progress-bar">
      <span
        role="progressbar"
        aria-label={ariaLabel}
        aria-valuemin={0}
        aria-valuemax={max}
        aria-valuenow={Math.round(clamped)}
        className={cn("h-1.5 w-full overflow-hidden rounded-full bg-(--surf2)", trackClassName)}
      >
        <span
          className={cn("block h-full rounded-full transition-[width]", tint)}
          style={{ width: `${percent}%` }}
        />
      </span>
      {label ? (
        <span className="shrink-0 font-tabular text-2xs whitespace-nowrap text-(--text2)">
          {label}
        </span>
      ) : null}
    </span>
  )
}
