import { Link } from "@tanstack/react-router"
import { BotOffIcon, LockIcon, RefreshCwIcon } from "lucide-react"
import { useEffect, useState } from "react"

import { Sparkline, StatTile } from "@/components/data/charts"
import { EnvTag } from "@/components/data/Env"
import { TargetMenu } from "@/components/menus/TargetMenu"
import { Button } from "@/components/ui/button"
import { useConnectionLockState } from "@/data/safety/queries"
import { useRefetchInterval } from "@/data/settings/useRefreshSettings"
import { endpointLine, readRedisEndpoint } from "@/domain/bullmqEndpoint"
import { formatCount } from "@/domain/format"
import {
  EditConnectionDialog,
  plainConnectionError,
  useConnectionMenu,
} from "@/features/connections"
import { effectiveMcpReadonly } from "@/features/mcp"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import { cn } from "@/lib/utils"
import { HOME_HEALTH_FLOOR_MS } from "./useConnectionHealth"
import type { FleetRow } from "./useFleet"

const sum = (xs: Array<number | undefined>) => xs.reduce<number>((a, b) => a + (b ?? 0), 0)

/** The whole fleet in four figures. */
export function FleetTiles({ rows }: { rows: FleetRow[] }) {
  const offline = rows.filter((r) => r.error !== null).length
  const reachable = rows.filter((r) => r.health)
  const failedHour = sum(reachable.map((r) => r.failedLastHour))
  const knownWorkers = reachable.filter((r) => r.workers !== undefined)
  // Nothing read yet: a placeholder, never "0 failed".
  const loading = reachable.length === 0 && offline < rows.length
  return (
    <section aria-label="The fleet at a glance" className="grid grid-cols-2 gap-3 md:grid-cols-4">
      <StatTile
        label="Connections"
        value={rows.length}
        sub={offline > 0 ? `${offline} offline` : loading ? "checking" : "all reachable"}
        tone={offline > 0 ? "danger" : undefined}
        testId="fleet-tile-connections"
      />
      <StatTile
        label="Failed"
        value={loading ? "-" : formatCount(sum(reachable.map((r) => r.health?.failed)))}
        sub={
          loading
            ? "loading"
            : failedHour > 0
              ? `+${formatCount(failedHour)} in the last hour`
              : "none in the last hour"
        }
        tone={failedHour > 0 ? "danger" : undefined}
        testId="fleet-tile-failed"
      />
      <StatTile
        label="Waiting"
        value={loading ? "-" : formatCount(sum(reachable.map((r) => r.health?.waiting)))}
        sub={loading ? "loading" : "across every queue"}
        testId="fleet-tile-waiting"
      />
      <StatTile
        label="Workers"
        value={knownWorkers.length > 0 ? formatCount(sum(knownWorkers.map((r) => r.workers))) : "-"}
        sub={
          loading
            ? "loading"
            : knownWorkers.length < reachable.length
              ? reachable.length - knownWorkers.length === 1
                ? "1 connection does not list its clients"
                : `${reachable.length - knownWorkers.length} connections do not list their clients`
              : "connected"
        }
        testId="fleet-tile-workers"
      />
    </section>
  )
}

// Narrow: connection, latency, waiting, failed. From 48 rem of table the
// trend, queues, workers and protection come in, never squeezed until clipped.
const COLS =
  "grid grid-cols-[minmax(9rem,2fr)_4.5rem_4.5rem_4.5rem] items-center gap-x-4 @3xl:grid-cols-[minmax(12rem,2fr)_minmax(7rem,1fr)_4rem_5rem_5rem_5rem_minmax(7rem,1fr)]"
/** A column that only shows in a wide table. */
const WIDE = "hidden @3xl:block"

/** One group's connections as table rows. */
export function FleetTable({
  rows,
  lastOpenedId,
  readonlyDefault,
}: {
  rows: FleetRow[]
  lastOpenedId: string | null
  readonlyDefault: boolean
}) {
  return (
    <div className="@container rounded-card border border-(--line)">
      {/* Column labels for the eye; each row reads on its own. */}
      <div
        aria-hidden="true"
        className={cn(
          COLS,
          "border-b border-(--line) px-3 py-2 text-2xs font-semibold tracking-wide text-(--text2) uppercase",
        )}
      >
        <span>Connection</span>
        <span>Health</span>
        <span className={cn(WIDE, "text-right")}>Queues</span>
        <span className="text-right">Waiting</span>
        <span className="text-right">Failed</span>
        <span className={cn(WIDE, "text-right")}>Workers</span>
        <span className={WIDE}>Protection</span>
      </div>
      <ul aria-label="Connections">
        {rows.map((row) => (
          <FleetRowView
            key={row.connection.id}
            row={row}
            lastOpened={row.connection.id === lastOpenedId}
            agentsReadonly={effectiveMcpReadonly(row.connection, readonlyDefault)}
          />
        ))}
      </ul>
    </div>
  )
}

const NUM = "text-right font-tabular text-sm"
const num = (n: number | undefined, tone?: string) =>
  n === undefined ? (
    <span className={cn(NUM, "text-(--text2)")}>-</span>
  ) : (
    <span className={cn(NUM, n === 0 ? "text-(--dim)" : (tone ?? "text-(--text)"))}>
      {formatCount(n)}
    </span>
  )

function FleetRowView({
  row,
  lastOpened,
  agentsReadonly,
}: {
  row: FleetRow
  lastOpened: boolean
  agentsReadonly: boolean
}) {
  const aiOn = useAiEnabled()
  const { connection: c, health, error } = row
  const redis = readRedisEndpoint(c)
  const menu = useConnectionMenu(c)
  const { data: lock } = useConnectionLockState(c.id)
  const [editing, setEditing] = useState(false)
  const offline = error !== null && !health
  return (
    <>
      <TargetMenu
        actions={menu.actions}
        render={
          <li
            className={cn(
              "border-b border-(--line) px-3 py-2 last:border-b-0",
              offline && "bg-(--danger)/5",
            )}
            data-testid="fleet-row"
            data-connection-id={c.id}
            data-offline={offline ? "true" : undefined}
          />
        }
      >
        <div className={COLS}>
          <span className="flex min-w-0 items-start gap-2">
            <span className="flex min-w-0 flex-col">
              <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
                <Link
                  to="/connections/$connectionId"
                  params={{ connectionId: c.id }}
                  className="font-medium break-words hover:underline"
                >
                  {c.name}
                </Link>
                <EnvTag color={c.envColor} label={c.envLabel} />
                {lastOpened ? (
                  <span className="text-2xs font-normal text-(--text2)">last opened</span>
                ) : null}
              </span>
              <span className="font-tabular text-xs wrap-anywhere text-(--text2)">
                {endpointLine(c) ?? c.backend}
              </span>
            </span>
          </span>
          <span className="flex min-w-0 items-center gap-2" data-testid="fleet-health">
            {offline ? (
              <span className="text-xs font-medium text-(--state-failed-text)">offline</span>
            ) : (
              <>
                <span className="w-12 shrink-0 font-tabular text-xs text-(--text2)">
                  {row.latencyMs !== undefined ? `${row.latencyMs} ms` : "-"}
                </span>
                <Sparkline
                  values={row.latency}
                  label={`${c.name} latency`}
                  color="var(--state-active)"
                  className="hidden min-w-0 @3xl:block"
                />
              </>
            )}
          </span>
          <span className={cn(WIDE, "text-right")}>{num(health?.queues)}</span>
          <span className="text-right">{num(health?.waiting)}</span>
          <span className="text-right">{num(health?.failed, "text-(--state-failed-text)")}</span>
          <span className={cn(WIDE, "text-right")}>{num(row.workers)}</span>
          <span className="hidden min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-(--text2) @3xl:flex">
            {lock === "locked" ? (
              <span className="inline-flex items-center gap-1 text-(--state-delayed-text)">
                <LockIcon className="size-3" aria-hidden="true" />
                Read-only
              </span>
            ) : lock === "unlocked" && c.envColor === "env-production" ? (
              <span>Unlocked</span>
            ) : null}
            {aiOn && agentsReadonly ? (
              <span className="inline-flex items-center gap-1">
                <BotOffIcon className="size-3" aria-hidden="true" />
                Agents read-only
              </span>
            ) : null}
            {redis?.tls ? <span>TLS</span> : null}
            {redis?.sshTunnel ? <span>SSH</span> : null}
          </span>
        </div>
        {offline ? <OfflineLine row={row} onEdit={() => setEditing(true)} /> : null}
      </TargetMenu>
      {menu.dialogs}
      <EditConnectionDialog connection={c} open={editing} onOpenChange={setEditing} />
    </>
  )
}

/** Why it cannot be reached, when Ranch tries again, and what to do now. */
function OfflineLine({ row, onEdit }: { row: FleetRow; onEdit: () => void }) {
  const interval = useRefetchInterval(HOME_HEALTH_FLOOR_MS)
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1_000)
    return () => window.clearInterval(id)
  }, [])
  const next =
    typeof interval === "number" && row.errorAt > 0
      ? Math.max(0, Math.ceil((row.errorAt + interval - now) / 1000))
      : null
  return (
    <div
      className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 pl-6 text-sm"
      data-testid="fleet-offline"
    >
      <span
        className="min-w-0 flex-1 break-words text-(--state-failed-text)"
        title={row.error ?? undefined}
      >
        {plainConnectionError(row.error ?? "", row.connection)}
      </span>
      {next !== null ? (
        <span className="text-xs text-(--text2)" data-testid="fleet-next-try">
          {next === 0 ? "trying again" : `next try in ${next} s`}
        </span>
      ) : null}
      <Button
        type="button"
        size="sm"
        variant="default"
        onClick={row.refetch}
        className="gap-1.5"
        data-testid="fleet-retry"
      >
        <RefreshCwIcon className="size-3.5" aria-hidden="true" />
        Retry now
      </Button>
      <Button type="button" size="sm" variant="ghost" onClick={onEdit} data-testid="fleet-edit">
        Edit
      </Button>
    </div>
  )
}
