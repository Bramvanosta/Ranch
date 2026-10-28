import type { MetricsRange } from "@/lib/tauri/bindings"

/** The eight windows, from 5 minutes to 7 days. Mirrors `MetricsRange` in Rust. */
export const METRICS_RANGES: { value: MetricsRange; label: string }[] = [
  { value: "5m", label: "Last 5 min" },
  { value: "15m", label: "Last 15 min" },
  { value: "30m", label: "Last 30 min" },
  { value: "1h", label: "Last hour" },
  { value: "6h", label: "Last 6 hours" },
  { value: "24h", label: "Last 24 hours" },
  { value: "3d", label: "Last 3 days" },
  { value: "7d", label: "Last 7 days" },
]

export const METRICS_RANGE_VALUES = METRICS_RANGES.map((r) => r.value) as [
  MetricsRange,
  ...MetricsRange[],
]

export const DEFAULT_METRICS_RANGE: MetricsRange = "24h"

export function rangeLabel(range: MetricsRange): string {
  return METRICS_RANGES.find((r) => r.value === range)?.label ?? range
}

/** x-axis tick for a bucket start: time of day for short windows, day + hour for long ones. */
export function formatBucketTick(startMs: number, range: MetricsRange): string {
  const d = new Date(startMs)
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
  if (range === "3d" || range === "7d") {
    return `${d.toLocaleDateString([], { weekday: "short" })} ${d.toLocaleTimeString([], { hour: "2-digit" })}`
  }
  return time
}
