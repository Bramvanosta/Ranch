import type { MetricsRange, Throughput } from "@/lib/tauri/bindings"

/** The Timeline's ranges. */
export const TIMELINE_RANGES = ["1h", "6h", "24h", "7d"] as const satisfies readonly MetricsRange[]
export type TimelineRange = (typeof TIMELINE_RANGES)[number]
export interface Span {
  startMs: number
  endMs: number
}

/** The visible range: from the first bucket to the end of the last. */
export function spanOf(t: Throughput): Span {
  const first = t.bucketStartsMs[0] ?? Date.now() - t.bucketMs
  const last = t.bucketStartsMs[t.bucketStartsMs.length - 1] ?? first
  return { startMs: first, endMs: last + t.bucketMs }
}

/** 0–1 along the axis. */
export const fraction = (span: Span, ms: number) =>
  Math.min(1, Math.max(0, (ms - span.startMs) / Math.max(1, span.endMs - span.startMs)))

export const timeAt = (span: Span, f: number) =>
  Math.round(span.startMs + Math.min(1, Math.max(0, f)) * (span.endMs - span.startMs))

/** About six labelled ticks on round times. */
export function ticks(span: Span): number[] {
  const len = span.endMs - span.startMs
  const steps = [5, 10, 15, 30, 60, 120, 180, 360, 720, 1440].map((m) => m * 60_000)
  const step = steps.find((s) => len / s <= 7) ?? steps[steps.length - 1]
  const out: number[] = []
  for (let t = Math.ceil(span.startMs / step) * step; t <= span.endMs; t += step) out.push(t)
  return out
}

export function tickLabel(ms: number, range: TimelineRange): string {
  const d = new Date(ms)
  return range === "7d"
    ? d.toLocaleDateString([], { weekday: "short", day: "numeric" })
    : d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
}

export interface Marker {
  key: string
  atMs: number
  label: string
  kind: "audit" | "agent"
  title: string
}

/**
 * Markers on rows so labels never cover each other: a marker goes on the
 * first row whose last label ends before it starts. `widthOf` is the label's
 * share of the axis (0–1).
 */
export function placeMarkers(
  markers: Marker[],
  span: Span,
  widthOf: (m: Marker) => number,
): Array<Marker & { x: number; row: number }> {
  const ends: number[] = []
  return [...markers]
    .sort((a, b) => a.atMs - b.atMs)
    .map((m) => {
      const x = fraction(span, m.atMs)
      let row = ends.findIndex((end) => end <= x)
      if (row === -1) {
        row = ends.length
        ends.push(0)
      }
      ends[row] = x + widthOf(m)
      return { ...m, x, row }
    })
}

export function formatWait(ms: number | null): string {
  if (ms === null) return "—"
  if (ms < 1000) return `${ms} ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`
  return `${Math.round(ms / 60_000)} min`
}

export interface Moment {
  from: number
  to: number
}

/**
 * The moment with the most failures: the worst bucket, widened to the
 * neighbours that failed at least half as much (three at most each side).
 * `null` when nothing failed in the range (or no queue records metrics).
 */
export function worstMoment(t: Throughput): Moment | null {
  let peak = -1
  let at = -1
  t.failed.forEach((f, i) => {
    if (f > peak) {
      peak = f
      at = i
    }
  })
  if (peak <= 0) return null
  let left = at
  let right = at
  while (left > 0 && at - left < 3 && (t.failed[left - 1] ?? 0) >= peak / 2) left--
  while (right < t.failed.length - 1 && right - at < 3 && (t.failed[right + 1] ?? 0) >= peak / 2)
    right++
  return {
    from: t.bucketStartsMs[left] ?? 0,
    to: (t.bucketStartsMs[right] ?? 0) + t.bucketMs,
  }
}

/** The latest twenty-fourth of the range: what is shown when nothing stands out. */
export function latestMoment(span: Span): Moment {
  return { from: Math.round(span.endMs - (span.endMs - span.startMs) / 24), to: span.endMs }
}

export interface QueueRow {
  name: string
  completed: number[]
  failed: number[]
  done: number
  failures: number
}

/**
 * The queues that record metrics, the most failures first (then the
 * busiest), each with its series; `scale` is the highest completed + failed
 * of any queue in any bucket — one scale for every row.
 */
export function queueRows(t: Throughput): { rows: QueueRow[]; scale: number } {
  const rows = t.queues
    .filter((q) => !q.metricsOff)
    .map((q) => ({
      name: q.queueRef.name,
      completed: q.completed,
      failed: q.failed,
      done: q.completed.reduce((a, b) => a + b, 0),
      failures: q.failed.reduce((a, b) => a + b, 0),
    }))
    .sort((a, b) => b.failures - a.failures || b.done - a.done || a.name.localeCompare(b.name))
  const scale = Math.max(
    1,
    ...rows.flatMap((r) => r.completed.map((c, i) => c + (r.failed[i] ?? 0))),
  )
  return { rows, scale }
}
