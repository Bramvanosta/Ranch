import type { MetricsRange } from "@/lib/tauri/bindings"

import { METRICS_RANGES } from "./ranges"

interface TimeRangeSelectProps {
  value: MetricsRange
  onChange: (range: MetricsRange) => void
}

/** Metrics window selector. A native select: keyboard + a11y for free. */
export function TimeRangeSelect({ value, onChange }: TimeRangeSelectProps) {
  return (
    <select
      aria-label="Time range"
      value={value}
      onChange={(e) => onChange(e.target.value as MetricsRange)}
      className="h-8 rounded-md border border-(--line) bg-transparent px-2 text-xs outline-none focus-visible:ring-2 focus-visible:ring-(--accent)/50"
      data-testid="metrics-range-select"
    >
      {METRICS_RANGES.map((r) => (
        <option key={r.value} value={r.value}>
          {r.label}
        </option>
      ))}
    </select>
  )
}
