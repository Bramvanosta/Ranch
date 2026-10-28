import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"

import {
  intervalLabel,
  isRefreshInterval,
  REFRESH_INTERVALS_MS,
} from "@/data/settings/refreshSettings"
import { useRefreshSettings } from "@/data/settings/useRefreshSettings"

/**
 * Settings › Refresh: how often Ranch asks Redis again.
 * On a pay-per-command Redis, every poll is billed; on a busy one, 1 s is
 * what you want.
 */
export function RefreshSection() {
  const { settings, update } = useRefreshSettings()
  return (
    <section className="flex flex-col gap-4" data-testid="settings-refresh">
      <header>
        <h2 className="text-lg font-semibold">Refresh</h2>
        <p className="text-sm text-(--text2)">
          How often the queue counters, dashboards and lists ask Redis again while you look at them.
          Every view also has a Refresh button.
        </p>
      </header>

      <div className="flex items-center gap-3">
        <Switch
          id="refresh-auto"
          checked={settings.auto}
          onCheckedChange={(auto) => update({ ...settings, auto })}
          data-testid="refresh-auto"
        />
        <Label htmlFor="refresh-auto">Refresh automatically</Label>
      </div>

      <div className="flex items-center gap-3">
        <Label htmlFor="refresh-interval" className={settings.auto ? "" : "text-(--text2)"}>
          Every
        </Label>
        <select
          id="refresh-interval"
          value={settings.intervalMs}
          disabled={!settings.auto}
          onChange={(e) => {
            const ms = Number(e.target.value)
            if (isRefreshInterval(ms)) update({ ...settings, intervalMs: ms })
          }}
          data-testid="refresh-interval"
          className="h-9 rounded-md border border-(--line) bg-transparent px-2 text-sm disabled:opacity-50 dark:bg-(--field)/30"
        >
          {REFRESH_INTERVALS_MS.map((ms) => (
            <option key={ms} value={ms}>
              {intervalLabel(ms)}
            </option>
          ))}
        </select>
      </div>

      <p className="max-w-prose text-xs text-(--text2)">
        Heavier views never go faster than they were built for (dashboards 5 s, the cross-queue
        Failed view 10 s). Notifications on thresholds and agent approval requests keep their own
        pace, so they still reach you with auto-refresh off.
      </p>
    </section>
  )
}
