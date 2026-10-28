import { useQueries } from "@tanstack/react-query"
import { Link } from "@tanstack/react-router"
import { TriangleAlertIcon, XIcon } from "lucide-react"
import { useEffect, useMemo, useState } from "react"
import { useJobStates } from "@/capabilities"
import { StatTile } from "@/components/data/charts"
import { SectionHeader } from "@/components/data/SectionHeader"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useLatestJobsOf } from "@/data/jobs/queries"
import { useQueues } from "@/data/queues/queries"
import { useRefetchInterval } from "@/data/settings/useRefreshSettings"
import { workerKeys } from "@/data/workers/keys"
import { fetchWorkers, WORKERS_REFRESH_MS } from "@/data/workers/queries"
import { formatAge } from "@/domain/formatAge"
import { inLine } from "@/domain/stateCounts"
import { useSharedPolls } from "@/lib/polling/sharedPoll"
import type { Queue, QueueId, QueueWorker } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"
import { libraryVersions, useLastWorkerSeen, type WorkerState, workerState } from "./workerState"

export interface WorkerRow {
  queue: Queue
  worker: QueueWorker
  state: WorkerState
}

export const workerId = (queue: QueueId, worker: Pick<QueueWorker, "id">) => `${queue}/${worker.id}`

const STATE: Record<WorkerState, { label: string; className: string }> = {
  working: { label: "Working", className: "text-(--state-active-text)" },
  idle: { label: "Idle", className: "text-(--text2)" },
  paused: { label: "Paused", className: "text-(--state-delayed-text)" },
}

/** Every queue's workers (`list_workers`), with whether this Redis tells. */
export function useConnectionWorkers(connectionId: string) {
  const queues = useQueues(connectionId).data
  const interval = useRefetchInterval(WORKERS_REFRESH_MS)
  // One self-paced poller per query (see `useSharedPoll`).
  useSharedPolls(
    (queues ?? []).map((q) => workerKeys.list(connectionId, q.id)),
    interval,
  )
  const lists = useQueries({
    queries: (queues ?? []).map((q) => ({
      queryKey: workerKeys.list(connectionId, q.id),
      queryFn: () => fetchWorkers(connectionId, q.id),
      refetchInterval: false as const,
    })),
  })
  // `useQueries` returns a new array on every render: key the memo on the data.
  const version = lists.map((l) => l.dataUpdatedAt).join(",")
  // biome-ignore lint/correctness/useExhaustiveDependencies: `version` stands for `lists`.
  const groups = useMemo(
    () =>
      (queues ?? []).map((queue, i) => {
        const data = lists[i]?.data
        return {
          queue,
          supported: data?.supported ?? true,
          unsupportedReason: data?.unsupportedReason ?? null,
          workers: (data?.workers ?? []).map((worker) => ({
            queue,
            worker,
            state: workerState(worker, queue.isPaused),
          })),
        }
      }),
    [queues, version],
  )
  return {
    groups,
    loading: !queues || lists.some((l) => l.isLoading),
    supported: groups.every((g) => g.supported),
    unsupportedReason: groups.find((g) => !g.supported)?.unsupportedReason ?? null,
  }
}

/**
 * Who consumes the connection's queues: tiles
 * (connected, working, queues without a worker, client libraries), a banner
 * for each queue with waiting jobs and nobody to take them, then the
 * workers grouped by queue. An older library version is highlighted.
 */
export function ConnectionWorkers({
  connectionId,
  selected,
  onSelect,
}: {
  connectionId: string
  selected: string | null
  onSelect: (id: string) => void
}) {
  const { groups, loading, supported, unsupportedReason } = useConnectionWorkers(connectionId)
  const states = useJobStates(connectionId)
  const lastSeen = useLastWorkerSeen()
  const [now, setNow] = useState(() => Date.now())

  // Remember when each queue last had a worker (Redis forgets a client that left).
  useEffect(() => {
    const t = Date.now()
    setNow(t)
    for (const g of groups) {
      if (g.workers.length > 0) lastSeen.mark(`${connectionId}/${g.queue.name}`, t)
    }
  }, [groups, connectionId, lastSeen.mark])

  if (loading) {
    return (
      <div className="flex flex-col gap-2" role="status" aria-label="Loading workers">
        {[0, 1, 2].map((i) => (
          <Skeleton key={i} className="h-9 w-full" />
        ))}
      </div>
    )
  }
  if (!supported) {
    return (
      <div
        className="rounded-card border border-(--line) bg-(--surf) p-4 text-sm"
        data-testid="workers-unsupported"
      >
        <p className="font-medium">This Redis does not list its clients.</p>
        <p className="mt-1 text-xs text-(--text2)">
          Workers are read from <code className="font-mono">CLIENT LIST</code>, which this server
          refuses
          {unsupportedReason ? `: ${unsupportedReason}` : "."}
        </p>
      </div>
    )
  }

  const waitingOf = (q: Queue) => inLine(q.counts, states)
  const all = groups.flatMap((g) => g.workers)
  const libs = libraryVersions(all.map((w) => w.worker.library))
  const orphans = groups.filter(
    (g) => g.workers.length === 0 && !g.queue.isPaused && waitingOf(g.queue) > 0,
  )
  const without = groups.filter((g) => g.workers.length === 0).length

  return (
    <div className="flex flex-col gap-5" data-testid="connection-workers">
      <section aria-label="Workers at a glance" className="grid grid-cols-2 gap-3 md:grid-cols-4">
        <StatTile
          label="Connected"
          value={all.length}
          sub="workers on this connection"
          testId="workers-tile-connected"
        />
        <StatTile
          label="Working"
          value={all.filter((w) => w.state === "working").length}
          sub="on a job right now"
          testId="workers-tile-working"
        />
        <StatTile
          label="Queues without worker"
          value={without}
          sub={
            orphans.length > 0 ? `${orphans.length} with jobs waiting` : "none with jobs waiting"
          }
          tone={orphans.length > 0 ? "danger" : undefined}
          testId="workers-tile-without"
        />
        <StatTile
          label="Client libraries"
          value={libs.distinct.length}
          sub={
            libs.older.size > 0
              ? `older: ${[...libs.older].join(", ")}`
              : libs.distinct.join(", ") || "not reported"
          }
          tone={libs.older.size > 0 ? "warning" : undefined}
          testId="workers-tile-libraries"
        />
      </section>

      {orphans.map((g) => {
        const seen = lastSeen.seen.get(`${connectionId}/${g.queue.name}`)
        return (
          <div
            key={g.queue.name}
            role="alert"
            className="flex flex-wrap items-center gap-3 rounded-card border border-(--danger)/40 bg-(--danger)/10 px-3 py-2 text-sm"
            data-testid="workers-orphan"
          >
            <TriangleAlertIcon
              className="size-4 shrink-0 text-(--state-failed-text)"
              aria-hidden="true"
            />
            <span className="min-w-0 flex-1 break-words">
              <span className="font-medium">{g.queue.name}</span> has{" "}
              {waitingOf(g.queue).toLocaleString()} {waitingOf(g.queue) === 1 ? "job" : "jobs"}{" "}
              waiting and no worker
              {" · "}
              <span className="text-(--text2)">
                {seen
                  ? `last worker seen ${formatAge(Math.max(now - seen, 0))} ago`
                  : "no worker seen since Ranch opened"}
              </span>
            </span>
            <Link
              to="/connections/$connectionId/queues/$queueId"
              params={{ connectionId, queueId: g.queue.id }}
              className="text-xs underline-offset-2 hover:underline"
            >
              Open the queue
            </Link>
          </div>
        )
      })}

      {groups
        .filter((g) => g.workers.length > 0)
        .map((g) => (
          <section key={g.queue.name} aria-label={g.queue.name} className="flex flex-col gap-2">
            <SectionHeader title={g.queue.name} count={g.workers.length} level={3} />
            <ul className="divide-y divide-(--line) rounded-card border border-(--line)">
              {g.workers.map((w) => {
                const id = workerId(g.queue.id, w.worker)
                const older = w.worker.library !== null && libs.older.has(w.worker.library)
                return (
                  <li key={id}>
                    <button
                      type="button"
                      onClick={() => onSelect(id)}
                      aria-pressed={selected === id}
                      className={cn(
                        "grid w-full grid-cols-[minmax(8rem,1.2fr)_minmax(7rem,1fr)_5rem_5rem_minmax(7rem,1fr)] items-center gap-3 px-3 py-2 text-left text-sm hover:bg-(--surf2) focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-(--accent)/60",
                        selected === id && "bg-(--sel)",
                      )}
                      data-testid="worker-row"
                    >
                      <span className="min-w-0 font-medium break-words">
                        {w.worker.name ?? `client ${w.worker.id}`}
                      </span>
                      <span className="min-w-0 font-tabular text-xs wrap-anywhere text-(--text2)">
                        {w.worker.addr}
                      </span>
                      <span className="font-tabular text-xs text-(--text2)">
                        idle {formatAge(w.worker.idleSecs * 1000)}
                      </span>
                      <span className={cn("text-xs", STATE[w.state].className)}>
                        {STATE[w.state].label}
                      </span>
                      <span
                        className={cn(
                          "min-w-0 font-tabular text-xs break-words",
                          older ? "text-(--state-delayed-text)" : "text-(--text2)",
                        )}
                        data-older={older ? "true" : undefined}
                        data-testid="worker-library"
                      >
                        {w.worker.library ?? "-"}
                        {older ? " · older" : ""}
                      </span>
                    </button>
                  </li>
                )
              })}
            </ul>
          </section>
        ))}

      {all.length === 0 && orphans.length === 0 ? (
        <p className="rounded-card border border-(--line) p-6 text-center text-sm text-(--text2)">
          No worker is connected to these queues.
        </p>
      ) : null}
    </div>
  )
}

/** A worker beside the list: the jobs it processed lately (`pb`), its raw `CLIENT LIST` fields. */
export function WorkerInspector({
  connectionId,
  id,
  onClose,
}: {
  connectionId: string
  id: string
  onClose: () => void
}) {
  const { groups } = useConnectionWorkers(connectionId)
  const row = groups.flatMap((g) => g.workers).find((w) => workerId(w.queue.id, w.worker) === id)
  const queueId = row?.queue.id
  const queueIds = useMemo(() => (queueId ? [queueId] : []), [queueId])
  const recent = useLatestJobsOf(connectionId, queueIds)
  if (!row || !queueId) return null
  const name = row.worker.name
  const processed = (recent.get(queueId) ?? []).filter(
    (j) => name !== null && j.processedBy === name,
  )
  const now = Date.now()
  return (
    <div className="flex min-h-0 flex-col" data-testid="worker-inspector">
      <div className="flex items-center justify-end border-b border-(--line) px-2 py-1">
        <Button variant="ghost" size="icon-sm" aria-label="Close the inspector" onClick={onClose}>
          <XIcon aria-hidden="true" />
        </Button>
      </div>
      <div className="flex flex-col gap-5 p-4">
        <header className="flex flex-col gap-1">
          <h2 className="text-lg font-semibold break-words">{name ?? `client ${row.worker.id}`}</h2>
          <p className="text-sm text-(--text2)">
            on {row.queue.name} · {STATE[row.state].label} · connected{" "}
            {formatAge(row.worker.ageSecs * 1000)}
          </p>
        </header>
        <section aria-label="Recently processed" className="flex flex-col gap-2">
          <SectionHeader title="Recently processed" level={3} />
          {name === null ? (
            <p className="text-sm text-(--text2)">
              This worker has no name: jobs cannot be traced back to it.
            </p>
          ) : processed.length === 0 ? (
            <p className="text-sm text-(--text2)">None among the queue's recent jobs.</p>
          ) : (
            <ul className="flex flex-col gap-1 text-sm" data-testid="worker-processed">
              {processed.slice(0, 10).map((j) => (
                <li key={j.jobRef.id} className="flex flex-wrap items-baseline gap-x-2">
                  <Link
                    to="/connections/$connectionId/queues/$queueId"
                    params={{ connectionId, queueId }}
                    search={{ job: j.jobRef.id }}
                    className="font-tabular text-xs wrap-anywhere hover:underline"
                  >
                    {j.name} #{j.jobRef.id}
                  </Link>
                  <span
                    className={
                      j.state === "failed"
                        ? "text-xs text-(--state-failed-text)"
                        : "text-xs text-(--text2)"
                    }
                  >
                    {j.state} ·{" "}
                    {formatAge(Math.max(now - (j.finishedOn ?? j.processedOn ?? j.timestamp), 0))}{" "}
                    ago
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>
        <section aria-label="Client fields" className="flex flex-col gap-2">
          <SectionHeader title="Client list" level={3} />
          <dl
            className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-0.5 font-tabular text-xs"
            data-testid="worker-raw"
          >
            {Object.entries(row.worker.raw).map(([k, v]) => (
              <div key={k} className="contents">
                <dt className="text-(--text2)">{k}</dt>
                <dd className="wrap-anywhere">{v}</dd>
              </div>
            ))}
          </dl>
        </section>
      </div>
    </div>
  )
}
