/**
 * auto-refresh, as data (pure, unit-tested).
 *
 * One user setting — on / off, and a base interval — drives every polled
 * view. Each view keeps a **floor**: the cheapest poll (queue counters) may
 * run every second, a heavier one (metrics, the cross-queue failed scan)
 * never faster than it was designed for, whatever the setting says.
 *
 * Not governed by it, on purpose: the reachability dot, the MCP approval
 * queue (a human is waiting on the other side) and the threshold watcher
 * (notifications must fire even with auto-refresh off).
 */
export const REFRESH_INTERVALS_MS = [1_000, 2_000, 5_000, 10_000, 30_000] as const
export type RefreshIntervalMs = (typeof REFRESH_INTERVALS_MS)[number]

export interface RefreshSettings {
  auto: boolean
  intervalMs: RefreshIntervalMs
}

/** What Ranch did before the setting existed. */
export const DEFAULT_REFRESH_SETTINGS: RefreshSettings = { auto: true, intervalMs: 1_000 }

export const KV_REFRESH_AUTO = "refresh.auto"
export const KV_REFRESH_INTERVAL = "refresh.intervalMs"

export function isRefreshInterval(value: number): value is RefreshIntervalMs {
  return (REFRESH_INTERVALS_MS as readonly number[]).includes(value)
}

/** From the two stored strings; anything unexpected falls back to the default. */
export function parseRefreshSettings(
  auto: string | null,
  interval: string | null,
): RefreshSettings {
  const parsed = Number(interval)
  return {
    auto: auto === null ? DEFAULT_REFRESH_SETTINGS.auto : auto !== "0",
    intervalMs: isRefreshInterval(parsed) ? parsed : DEFAULT_REFRESH_SETTINGS.intervalMs,
  }
}

/** TanStack Query's `refetchInterval`: `false` = do not poll. */
export function effectiveInterval(settings: RefreshSettings, floorMs: number): number | false {
  if (!settings.auto) return false
  return Math.max(settings.intervalMs, floorMs)
}

export function intervalLabel(ms: number): string {
  return ms % 1_000 === 0 ? `${ms / 1_000} s` : `${ms} ms`
}
