/**
 * Compact age formatter for job rows.
 *
 * Formats a millisecond duration as a tight string for the age cell:
 *   - 0–59s   → `Ns`
 *   - 1–59m   → `Nm`
 *   - 1–23h   → `Nh`
 *   - ≥24h    → `Nd`
 *
 * Negative durations (clock skew between the user's machine and the
 * Redis server) collapse to `now` rather than showing nonsensical
 * negative values.
 */
export function formatAge(deltaMs: number): string {
  if (!Number.isFinite(deltaMs) || deltaMs < 0) return "now"
  const s = Math.floor(deltaMs / 1000)
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m`
  const h = Math.floor(m / 60)
  if (h < 24) return `${h}h`
  const d = Math.floor(h / 24)
  return `${d}d`
}
