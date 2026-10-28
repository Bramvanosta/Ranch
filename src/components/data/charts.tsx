import type { ReactNode } from "react"

import { cn } from "@/lib/utils"

/**
 * Small charts next to the numbers. Hand-drawn SVG
 * and boxes on the design tokens, so they follow both themes; each fills
 * the width it is given and never draws outside it. "Metrics off" (a queue
 * that records no native metrics) is said, never drawn as zeros.
 */

/** A line over time, oldest first. `null` = metrics off. */
export function Sparkline({
  values,
  color = "var(--state-completed)",
  label,
  className,
}: {
  values: number[] | null
  color?: string
  /** What the line is, for screen readers ("Completed per hour"). */
  label: string
  className?: string
}) {
  const off = values === null
  const empty = !off && values.length === 0
  const summary = off
    ? `${label}: metrics off`
    : empty
      ? `${label}: no data`
      : `${label}: from ${values[0]} to ${values[values.length - 1]}, peak ${Math.max(...values)}`
  if (off || empty) {
    return (
      <span
        role="img"
        aria-label={summary}
        className={cn("block h-6 w-full border-b border-dashed border-(--line)", className)}
        data-testid="sparkline"
        data-state={off ? "off" : "empty"}
      />
    )
  }
  const max = Math.max(...values)
  const min = Math.min(...values)
  const span = max - min
  const n = values.length
  // A flat line sits in the middle when it is not zero, on the floor when it is.
  const y = (v: number) => (span === 0 ? (max === 0 ? 22 : 12) : 22 - ((v - min) / span) * 20)
  const x = (i: number) => (n === 1 ? 50 : (i / (n - 1)) * 100)
  const points = values.map((v, i) => `${x(i).toFixed(2)},${y(v).toFixed(2)}`).join(" ")
  return (
    <svg
      role="img"
      aria-label={summary}
      viewBox="0 0 100 24"
      preserveAspectRatio="none"
      className={cn("block h-6 w-full overflow-hidden", className)}
      data-testid="sparkline"
      data-state="data"
    >
      <polyline
        points={points}
        fill="none"
        stroke={color}
        strokeWidth={1.5}
        strokeLinejoin="round"
        strokeLinecap="round"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  )
}

/**
 * Buckets as cells: the darker, the busier; the red part of a cell is the
 * share that failed. `metricsOff` draws a hatched strip that says so.
 */
export function HeatStrip({
  completed,
  failed,
  metricsOff = false,
  label,
  className,
}: {
  completed: number[]
  failed: number[]
  metricsOff?: boolean
  label: string
  className?: string
}) {
  if (metricsOff) {
    return (
      <span
        role="img"
        aria-label={`${label}: metrics off`}
        className={cn(
          "flex h-4 w-full min-w-0 items-center justify-center rounded-tag bg-[repeating-linear-gradient(135deg,var(--line)_0_2px,transparent_2px_6px)] text-2xs text-(--text2)",
          className,
        )}
        data-testid="heat-strip"
        data-state="off"
      >
        metrics off
      </span>
    )
  }
  const n = Math.max(completed.length, failed.length)
  const totals = Array.from({ length: n }, (_, i) => (completed[i] ?? 0) + (failed[i] ?? 0))
  const max = Math.max(0, ...totals)
  const done = completed.reduce((a, b) => a + b, 0)
  const lost = failed.reduce((a, b) => a + b, 0)
  return (
    <span
      role="img"
      aria-label={`${label}: ${done} completed, ${lost} failed`}
      className={cn("flex h-4 w-full min-w-0 gap-px overflow-hidden rounded-tag", className)}
      data-testid="heat-strip"
      data-state={n === 0 ? "empty" : "data"}
    >
      {n === 0 ? <span className="h-full w-full bg-(--surf2)" /> : null}
      {totals.map((total, i) => {
        const failShare = total === 0 ? 0 : (failed[i] ?? 0) / total
        // Never fully transparent: an idle bucket is still a cell.
        const intensity = max === 0 ? 0 : 0.15 + 0.85 * (total / max)
        return (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: one cell per bucket, in order.
            key={i}
            className="relative h-full min-w-0 flex-1 bg-(--surf2)"
            data-testid="heat-cell"
          >
            {total > 0 ? (
              <span
                className="absolute inset-0 bg-(--state-completed)"
                style={{ opacity: intensity }}
              />
            ) : null}
            {failShare > 0 ? (
              <span
                className="absolute inset-x-0 bottom-0 bg-(--state-failed)"
                style={{ height: `${Math.max(failShare * 100, 25)}%` }}
                data-testid="heat-cell-failed"
              />
            ) : null}
          </span>
        )
      })}
    </span>
  )
}

export interface StackedSegment {
  key: string
  label: string
  value: number
  /** A CSS colour, e.g. `var(--state-waiting)`. */
  color: string
}

/** The mix of a whole: one bar, one segment per part, in proportion. */
export function StackedBar({
  segments,
  label,
  className,
}: {
  segments: StackedSegment[]
  label: string
  className?: string
}) {
  const total = segments.reduce((a, s) => a + Math.max(s.value, 0), 0)
  const parts = segments.filter((s) => s.value > 0)
  return (
    <span
      role="img"
      aria-label={
        total === 0
          ? `${label}: none`
          : `${label}: ${parts.map((s) => `${s.value} ${s.label.toLowerCase()}`).join(", ")}`
      }
      className={cn("flex h-2 w-full min-w-0 overflow-hidden rounded-full bg-(--surf2)", className)}
      data-testid="stacked-bar"
    >
      {parts.map((s) => (
        <span
          key={s.key}
          className="h-full min-w-0.5"
          style={{ width: `${(s.value / total) * 100}%`, background: s.color }}
          data-testid={`stacked-bar-${s.key}`}
        />
      ))}
    </span>
  )
}

/** A figure with its label, a line under it and, optionally, its trend. */
export function StatTile({
  label,
  value,
  sub,
  trend,
  trendColor,
  tone,
  testId,
  className,
}: {
  label: string
  value: ReactNode
  sub?: ReactNode
  /** Oldest first; `null` = metrics off; leave out for no sparkline. */
  trend?: number[] | null
  trendColor?: string
  /** The figure's colour when it needs attention. */
  tone?: "danger" | "warning"
  testId?: string
  className?: string
}) {
  return (
    <div
      className={cn(
        // A subgrid of the row of tiles: label, figure, sub-line and trend each
        // take one shared track, so a label that wraps to two lines moves the
        // figures of the whole row together instead of only its own.
        "row-span-4 grid min-w-0 grid-rows-subgrid gap-1 rounded-card border border-(--line) bg-(--surf) p-3",
        className,
      )}
      data-testid={testId ?? "stat-tile"}
    >
      <span className="text-2xs font-semibold tracking-wide break-words text-(--text2) uppercase">
        {label}
      </span>
      <span
        className={cn(
          "font-tabular text-2xl leading-tight font-semibold wrap-anywhere",
          tone === "danger" && "text-(--state-failed-text)",
          tone === "warning" && "text-(--state-delayed-text)",
        )}
        data-testid="stat-tile-value"
      >
        {value}
      </span>
      {sub ? (
        <span
          className="row-start-3 text-xs break-words text-(--text2)"
          data-testid="stat-tile-sub"
        >
          {sub}
        </span>
      ) : null}
      {trend !== undefined ? (
        <Sparkline
          values={trend}
          color={trendColor}
          label={label}
          className="row-start-4 self-end"
        />
      ) : null}
    </div>
  )
}
