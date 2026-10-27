import { useQuery, useQueryClient } from "@tanstack/react-query"
import { Link } from "@tanstack/react-router"
import { BellOffIcon } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"
import { ConfirmSheet } from "@/components/data/ConfirmSheet"
import { Sparkline } from "@/components/data/charts"
import { EnvDot } from "@/components/data/Env"
import { SectionHeader } from "@/components/data/SectionHeader"
import { Segmented } from "@/components/data/Segmented"
import { Toolbar } from "@/components/data/Toolbar"
import { Button, buttonVariants } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Skeleton } from "@/components/ui/skeleton"
import { EditConnectionDialog } from "@/features/connections/EditConnectionDialog"
import { plainConnectionError } from "@/features/connections/plainError"
import { formatAge } from "@/features/jobs/formatAge"
import { formatCount } from "@/features/metrics/format"
import { withOneShotUnlock } from "@/features/safety/oneShotUnlock"
import { Screen } from "@/layouts/Screen"
import { type AttentionItem, type Connection, commands, type JobRef } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import { ATTENTION_QUERY_KEY, isMuted, useAttentionMutes, useAttentionReport } from "./useAttention"

type EnvFilter = "all" | "production" | "staging"

const MUTE_FOR = [
  { label: "1 hour", ms: 3_600_000 },
  { label: "4 hours", ms: 4 * 3_600_000 },
  { label: "1 day", ms: 24 * 3_600_000 },
  { label: "1 week", ms: 7 * 24 * 3_600_000 },
]

const SECTIONS: Array<{ kind: AttentionItem["kind"]; title: string; empty: string }> = [
  { kind: "failing", title: "Failing", empty: "Nothing is failing." },
  { kind: "stuck", title: "Stuck", empty: "No queue is stuck." },
  { kind: "offline", title: "Offline", empty: "Every connection answers." },
]

/**
 * Everything that needs the user, across connections, in
 * one place: Failing (failure causes), Stuck (paused too long, or jobs
 * waiting with no worker) and Offline, most urgent first; each with where it
 * is, since when, its trend and what to do. Filters by environment; Mute
 * hides an item for a while ("Muted (n)" keeps them reachable).
 */
export function AttentionPage() {
  const report = useAttentionReport()
  const { mutes, mute, unmute } = useAttentionMutes()
  const [env, setEnv] = useState<EnvFilter>("all")
  const [hideLocal, setHideLocal] = useState(false)
  const [showMuted, setShowMuted] = useState(false)
  const [now] = useState(() => Date.now())

  const items = (report.data?.items ?? []).filter((i) => {
    if (env === "production" && i.envColor !== "env-production") return false
    if (env === "staging" && i.envColor !== "env-staging") return false
    if (hideLocal && i.envColor === "env-local") return false
    return true
  })
  const muted = items.filter((i) => isMuted(i, mutes))
  const open = items.filter((i) => !isMuted(i, mutes))

  return (
    <Screen
      toolbar={
        <Toolbar
          title="Needs attention"
          meta={
            <span className="text-sm text-(--text2)" data-testid="attention-count">
              {report.data
                ? `${open.length} ${open.length === 1 ? "item" : "items"} across ${report.data.connectionsChecked} connections`
                : "Looking"}
            </span>
          }
          actions={
            <>
              <Segmented
                label="Environment"
                value={env}
                onChange={setEnv}
                options={[
                  { value: "all", label: "All", testId: "attention-env-all" },
                  { value: "production", label: "Production", testId: "attention-env-production" },
                  { value: "staging", label: "Staging", testId: "attention-env-staging" },
                ]}
              />
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={hideLocal}
                  onChange={(e) => setHideLocal(e.target.checked)}
                  className="size-4"
                  data-testid="attention-hide-local"
                />
                Hide Local
              </label>
            </>
          }
        />
      }
      contentClassName="overflow-auto"
    >
      <div className="flex w-full flex-col gap-6" data-testid="attention-page">
        {report.isLoading ? (
          <div
            className="flex flex-col gap-2"
            role="status"
            aria-label="Looking at every connection"
          >
            {[0, 1, 2].map((i) => (
              <Skeleton key={i} className="h-12 w-full" />
            ))}
          </div>
        ) : report.error ? (
          <p role="alert" className="text-sm text-(--state-failed-text)">
            {report.error instanceof Error ? report.error.message : String(report.error)}
          </p>
        ) : (
          SECTIONS.map((section) => {
            const list = open.filter((i) => i.kind === section.kind)
            return (
              <section
                key={section.kind}
                aria-label={section.title}
                className="flex flex-col gap-2"
                data-testid={`attention-section-${section.kind}`}
              >
                <SectionHeader title={section.title} count={list.length} />
                {list.length === 0 ? (
                  <p className="rounded-card border border-(--line) px-3 py-3 text-sm text-(--text2)">
                    {section.empty}
                  </p>
                ) : (
                  <ul className="divide-y divide-(--line) rounded-card border border-(--line)">
                    {list.map((item) => (
                      <AttentionRow
                        key={item.key}
                        item={item}
                        now={now}
                        onMute={(ms) => mute(item.key, Date.now() + ms)}
                      />
                    ))}
                  </ul>
                )}
              </section>
            )
          })
        )}
        {muted.length > 0 ? (
          <section aria-label="Muted" className="flex flex-col gap-2">
            <button
              type="button"
              onClick={() => setShowMuted((v) => !v)}
              aria-expanded={showMuted}
              className="self-start text-sm text-(--text2) hover:text-(--text)"
              data-testid="attention-muted-toggle"
            >
              Muted ({muted.length})
            </button>
            {showMuted ? (
              <ul className="divide-y divide-(--line) rounded-card border border-(--line)">
                {muted.map((item) => (
                  <li
                    key={item.key}
                    className="flex flex-wrap items-center gap-3 px-3 py-2 text-sm"
                    data-testid="attention-muted-item"
                  >
                    <span className="min-w-0 flex-1 break-words text-(--text2)">
                      {item.connectionName}
                      {item.queueName ? ` / ${item.queueName}` : ""} · {describe(item)} · until{" "}
                      {new Date(mutes[item.key] ?? 0).toLocaleString()}
                    </span>
                    <Button size="sm" variant="ghost" onClick={() => unmute(item.key)}>
                      Unmute
                    </Button>
                  </li>
                ))}
              </ul>
            ) : null}
          </section>
        ) : null}
      </div>
    </Screen>
  )
}

function describe(item: AttentionItem): string {
  if (item.kind === "failing")
    return item.reason === "" ? "No failure reason recorded" : item.reason
  if (item.kind === "stuck") {
    return item.reason === "paused"
      ? `paused · ${formatCount(item.count)} waiting`
      : `no worker · ${formatCount(item.count)} waiting`
  }
  return "offline"
}

const OPEN = cn(buttonVariants({ variant: "default", size: "sm" }))

function AttentionRow({
  item,
  now,
  onMute,
}: {
  item: AttentionItem
  now: number
  onMute: (ms: number) => void
}) {
  const qc = useQueryClient()
  const [confirming, setConfirming] = useState<"retry" | "resume" | null>(null)
  const [locked, setLocked] = useState(false)
  const [busy, setBusy] = useState(false)
  const [editing, setEditing] = useState<Connection | null>(null)

  const ask = async (what: "retry" | "resume") => {
    const r = await commands.getConnectionLockState({ connectionId: item.connectionId })
    setLocked(r.status === "ok" && r.data === "locked")
    setConfirming(what)
  }
  const run = async () => {
    setBusy(true)
    try {
      const send = async () => {
        if (confirming === "resume" && item.queueName) {
          const r = await commands.resumeQueue({
            connectionId: item.connectionId,
            queueRef: { prefix: "bull", name: item.queueName },
          })
          if (r.status === "error") throw new Error(r.error.message)
          return
        }
        for (const [queue, jobs] of byQueue(item.jobs)) {
          const r = await commands.bulkRetryJobs({
            connectionId: item.connectionId,
            queueRef: jobs[0]?.queue ?? { prefix: "bull", name: queue },
            jobIds: jobs.map((j) => j.id),
          })
          if (r.status === "error") throw new Error(r.error.message)
        }
      }
      await (locked ? withOneShotUnlock(item.connectionId, send) : send())
      toast.success(
        confirming === "resume" ? `${item.queueName} resumed` : `${item.count} jobs retried`,
      )
      setConfirming(null)
      qc.invalidateQueries({ queryKey: ATTENTION_QUERY_KEY })
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }
  const openEdit = async () => {
    const r = await commands.listConnections()
    if (r.status === "ok") setEditing(r.data.find((c) => c.id === item.connectionId) ?? null)
  }

  return (
    <li
      className="flex flex-wrap items-center gap-3 px-3 py-2"
      data-testid={`attention-item-${item.kind}`}
    >
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex flex-wrap items-center gap-x-2 text-sm">
          <EnvDot color={item.envColor} label={item.envLabel} />
          <span className="font-medium break-words">{item.connectionName}</span>
          {item.queueName ? (
            <span className="font-tabular text-xs text-(--text2)">/ {item.queueName}</span>
          ) : null}
        </span>
        <span
          className={cn(
            "text-sm break-words",
            item.kind === "failing" ? "font-tabular" : "",
            item.kind === "offline" && "text-(--state-failed-text)",
          )}
        >
          {item.kind === "failing" ? (
            <>
              <span className="text-(--state-failed-text)">{formatCount(item.count)}</span>{" "}
              {describe(item)}
            </>
          ) : item.kind === "offline" ? (
            <OfflineReason item={item} />
          ) : (
            describe(item)
          )}
        </span>
        <span className="text-xs text-(--text2)">
          {item.sinceMs
            ? `since ${formatAge(Math.max(now - item.sinceMs, 0))} ago`
            : "since: not known"}
        </span>
      </span>
      {item.trend.length > 0 ? (
        <span className="w-28 shrink-0">
          <Sparkline
            values={item.trend}
            label="Failures over 24 hours"
            color="var(--state-failed)"
          />
        </span>
      ) : null}
      <span className="flex shrink-0 flex-wrap items-center gap-1.5">
        {item.kind !== "offline" ? (
          <ItemLink item={item} />
        ) : (
          <>
            <Button
              size="sm"
              variant="default"
              onClick={() => qc.invalidateQueries({ queryKey: ATTENTION_QUERY_KEY })}
              data-testid="attention-retry-now"
            >
              Retry now
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void openEdit()}
              data-testid="attention-edit"
            >
              Edit
            </Button>
          </>
        )}
        {item.kind === "failing" && item.jobs.length > 0 ? (
          <Button
            size="sm"
            variant="default"
            onClick={() => void ask("retry")}
            data-testid="attention-retry"
          >
            Retry {formatCount(item.jobs.length)}
          </Button>
        ) : null}
        {item.kind === "stuck" && item.reason === "paused" ? (
          <Button
            size="sm"
            variant="default"
            onClick={() => void ask("resume")}
            data-testid="attention-resume"
          >
            Resume
          </Button>
        ) : null}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button size="sm" variant="ghost" className="gap-1.5" data-testid="attention-mute">
              <BellOffIcon className="size-3.5" aria-hidden="true" />
              Mute
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {MUTE_FOR.map((m) => (
              <DropdownMenuItem key={m.label} onSelect={() => onMute(m.ms)}>
                For {m.label}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </span>
      <ConfirmSheet
        open={confirming !== null}
        onOpenChange={(o) => {
          if (!o) setConfirming(null)
        }}
        title={
          confirming === "resume"
            ? `Resume ${item.queueName}?`
            : `Retry ${formatCount(item.jobs.length)} jobs of this cause?`
        }
        description={confirming === "resume" ? "Workers take its jobs again." : describe(item)}
        removed={
          confirming === "retry"
            ? [{ label: "Failed jobs moved back to waiting", count: item.jobs.length }]
            : []
        }
        protectedName={locked ? (item.queueName ?? item.connectionName) : undefined}
        confirmLabel={
          confirming === "resume" ? "Resume" : `Retry ${formatCount(item.jobs.length)} jobs`
        }
        onConfirm={() => void run()}
        busy={busy}
        testId="attention-sheet"
        confirmTestId="attention-confirm"
      />
      {editing ? (
        <EditConnectionDialog
          connection={editing}
          open
          onOpenChange={(o) => !o && setEditing(null)}
        />
      ) : null}
    </li>
  )
}

/** Why it is offline, in plain words, with the connection's real host and port. */
function OfflineReason({ item }: { item: AttentionItem }) {
  const { data: connections } = useQuery({
    queryKey: ["connections"],
    queryFn: async () => {
      const r = await commands.listConnections()
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
  })
  const c = connections?.find((x) => x.id === item.connectionId)
  return <>{c ? plainConnectionError(item.reason, c) : item.reason}</>
}

function byQueue(jobs: JobRef[]): Map<string, Array<{ queue: JobRef["queue"]; id: string }>> {
  const out = new Map<string, Array<{ queue: JobRef["queue"]; id: string }>>()
  for (const j of jobs) {
    out.set(j.queue.name, [...(out.get(j.queue.name) ?? []), { queue: j.queue, id: j.id }])
  }
  return out
}

function ItemLink({ item }: { item: AttentionItem }) {
  if (item.queueName) {
    return (
      <Link
        to="/connections/$connectionId/queues/$queueName"
        params={{ connectionId: item.connectionId, queueName: item.queueName }}
        search={
          item.kind === "failing"
            ? { view: "jobs", state: "failed", group: "cause", page: 0 }
            : item.reason === "noWorker"
              ? { view: "workers" }
              : { view: "jobs", state: "waiting", page: 0 }
        }
        className={OPEN}
      >
        Open
      </Link>
    )
  }
  return (
    <Link
      to="/connections/$connectionId"
      params={{ connectionId: item.connectionId }}
      search={{ view: "failed" }}
      className={OPEN}
    >
      Open
    </Link>
  )
}
