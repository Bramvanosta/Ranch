import { cn } from "@/lib/utils"

/**
 * One row of a `TraceBar`: a span of time on the shared axis.
 *
 *   - `wait`: waiting for a worker (a thin neutral bar);
 *   - `work`: a worker on it, coloured by its outcome;
 *   - `backoff`: the pause BullMQ takes before the next attempt (hatched);
 *   - `pending`: what is still to come (dashed, up to now).
 */
export interface TraceRow {
  key: string
  label: string
  start: number
  end: number
  kind: "wait" | "work" | "backoff" | "pending"
  tone: "failed" | "completed" | "running" | "neutral"
  /** What the row says on its right: a duration, an outcome. */
  detail: string
  /** Placed from settings, not from stored times. */
  estimated?: boolean
}

const TONE: Record<TraceRow["tone"], string> = {
  failed: "bg-(--state-failed)",
  completed: "bg-(--state-completed)",
  running: "bg-(--state-active)",
  neutral: "bg-(--text2)/50",
}

/**
 * A job's life on one time axis, one row per wait,
 * attempt and backoff. Rows never clip their text: the label and the detail
 * sit beside the track, and a span too short to see still shows as a 2 px
 * mark.
 */
export function TraceBar({
  rows,
  start,
  end,
  className,
}: {
  rows: TraceRow[]
  start: number
  end: number
  className?: string
}) {
  const span = Math.max(end - start, 1)
  const pct = (t: number) => Math.min(Math.max(((t - start) / span) * 100, 0), 100)
  return (
    <ol
      className={cn("grid grid-cols-[auto_minmax(0,1fr)_auto] gap-x-3 gap-y-1.5", className)}
      data-testid="trace-bar"
    >
      {rows.map((row) => {
        const left = pct(row.start)
        const width = Math.max(pct(row.end) - left, 0)
        return (
          <li
            key={row.key}
            className="col-span-3 grid grid-cols-subgrid items-center text-xs"
            data-testid={`trace-row-${row.key}`}
            data-kind={row.kind}
            data-estimated={row.estimated ? "true" : undefined}
          >
            <span className="text-(--text2)">{row.label}</span>
            <span className="relative h-3 rounded-tag bg-(--surf2)" aria-hidden="true">
              <span
                className={cn(
                  "absolute top-0 h-full min-w-0.5 rounded-tag",
                  row.kind === "wait" && "top-1 h-1",
                  row.kind === "work" && TONE[row.tone],
                  row.kind === "wait" && TONE.neutral,
                  row.kind === "backoff" &&
                    "bg-[repeating-linear-gradient(135deg,var(--text2)_0_2px,transparent_2px_5px)] opacity-60",
                  row.kind === "pending" && "border border-dashed border-(--text2)/60",
                  row.estimated && row.kind === "work" && "opacity-70",
                )}
                style={{ left: `${left}%`, width: `${width}%` }}
              />
            </span>
            <span className="text-right font-tabular text-(--text)">
              {row.estimated && row.kind !== "work" ? "≈ " : ""}
              {row.detail}
            </span>
          </li>
        )
      })}
    </ol>
  )
}
