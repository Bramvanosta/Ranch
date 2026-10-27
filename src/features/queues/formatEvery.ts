/**
 * Format a millisecond interval as a compact human string
 * for the CRON view's Pattern column when the scheduler is fixed-interval
 * (`every`-based) rather than cron-pattern-based.
 *
 * Boundaries:
 *   - <1000ms      → `"{n}ms"`
 *   - 1s–59s       → `"{n}s"`
 *   - 1m–59m       → `"{n}m"`
 *   - 1h–23h       → `"{n}h"`
 *   - ≥1 day       → `"{n}d"`
 *
 * Negative or non-finite input collapses to `"—"`.
 */
export function formatEveryMs(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "—"
  if (ms < 1000) return `${Math.floor(ms)}ms`
  const s = Math.floor(ms / 1000)
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m`
  const h = Math.floor(m / 60)
  if (h < 24) return `${h}h`
  const d = Math.floor(h / 24)
  return `${d}d`
}
