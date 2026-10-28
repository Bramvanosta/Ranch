/**
 * Display formatters for the overview dashboard.
 *
 * Tabular-friendly: every formatter returns a short string that fits
 * inside MetricCard's 2xl value slot without wrapping at the typical
 * dashboard width (~280px column).
 */

/** Format a duration in ms as a compact human string. */
export function formatDurationMs(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "-"
  if (ms < 1_000) return `${Math.round(ms)} ms`
  const s = ms / 1_000
  if (s < 60) return `${s.toFixed(s < 10 ? 2 : 1)} s`
  const m = s / 60
  if (m < 60) return `${m.toFixed(m < 10 ? 1 : 0)} min`
  const h = m / 60
  return `${h.toFixed(h < 10 ? 1 : 0)} h`
}

/** Format a 0..1 ratio as a percentage with sensible precision. */
export function formatPercent(ratio: number): string {
  if (!Number.isFinite(ratio)) return "-"
  const pct = ratio * 100
  if (pct === 0) return "0%"
  if (pct < 1) return `${pct.toFixed(2)}%`
  if (pct < 10) return `${pct.toFixed(1)}%`
  return `${Math.round(pct)}%`
}

/** Format a per-hour throughput rate. */
/** Jobs per minute; switches to per hour when that reads better. */
export function formatThroughputPerMinute(perMinute: number): string {
  if (!Number.isFinite(perMinute) || perMinute <= 0) return "0/min"
  if (perMinute < 0.1) return formatThroughput(perMinute * 60)
  if (perMinute < 10) return `${perMinute.toFixed(1)}/min`
  if (perMinute < 1_000) return `${Math.round(perMinute)}/min`
  return `${(perMinute / 1_000).toFixed(1)}k/min`
}

export function formatThroughput(perHour: number): string {
  if (!Number.isFinite(perHour) || perHour <= 0) return "0/h"
  if (perHour < 10) return `${perHour.toFixed(1)}/h`
  if (perHour < 1_000) return `${Math.round(perHour)}/h`
  return `${(perHour / 1_000).toFixed(1)}k/h`
}

/** Format a count with thousand separators. */
export function formatCount(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return "0"
  return new Intl.NumberFormat("en-US").format(Math.round(n))
}

/** A memory size as the info strip shows it: "2.1 MB", "1.2 GB". */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${Math.round(bytes)} B`
  const units = ["KB", "MB", "GB", "TB"]
  let value = bytes / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit++
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`
}

/** Pretty-print valid JSON; leave anything else untouched. */
export function formatJson(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2)
  } catch {
    return text
  }
}
