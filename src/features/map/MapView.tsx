import { useQuery } from "@tanstack/react-query"
import { Link } from "@tanstack/react-router"
import { ScanIcon, XIcon } from "lucide-react"
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"

import { useCommandHandler } from "@/commands/handlers"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { formatAge } from "@/features/jobs/formatAge"
import { formatCount } from "@/features/metrics/format"
import { useQueues } from "@/features/queues/useQueues"
import { useRefetchInterval } from "@/features/settings/useRefreshSettings"
import { useConnectionWorkers } from "@/features/workers/ConnectionWorkers"
import { useSharedPoll } from "@/lib/polling/sharedPoll"
import { commands, type QueueWorker } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import { buildMap, failureTone, type MapModel, type MapQueue, NO_WORKER } from "./mapModel"

/** The Map reads every queue's flows and events: never faster than this. */
const MAP_REFRESH_FLOOR_MS = 30_000

type Selected = { kind: "queue" | "pool" | "noWorker"; id: string }

interface Anchor {
  left: number
  right: number
  y: number
}

const TONE_STROKE = {
  ok: "var(--state-completed)",
  warn: "var(--state-delayed)",
  danger: "var(--state-failed)",
} as const

const MIN_FIT = 0.5

const rate = (perMin: number) =>
  perMin === 0 ? "0" : perMin < 1 ? perMin.toFixed(2) : formatCount(Math.round(perMin))

/**
 * The Map tab: which queues feed which workers and where
 * it is red. Queues on the left, worker pools on the right; an edge's width
 * is the throughput, its colour the failure rate, dashed when paused or idle;
 * flow links between queues on the left; "No worker" where nobody consumes.
 */
export function MapView({
  connectionId,
  onlyActive,
  onOnlyActiveChange,
}: {
  connectionId: string
  /** "Only queues with activity" (in the URL: `all=1` turns it off). */
  onlyActive: boolean
  onOnlyActiveChange: (only: boolean) => void
}) {
  const [fit, setFit] = useState(false)
  const [selected, setSelected] = useState<Selected | null>(null)

  const queues = useQueues(connectionId).data
  const { groups } = useConnectionWorkers(connectionId)
  // The refresh setting (30 s floor: the Map reads every queue),
  // one self-paced poller per query.
  const mapInterval = useRefetchInterval(MAP_REFRESH_FLOOR_MS)
  useSharedPoll(["throughput", connectionId, "1h"], mapInterval)
  useSharedPoll(["topology", connectionId], mapInterval)
  const throughput = useQuery({
    queryKey: ["throughput", connectionId, "1h"],
    queryFn: async () => {
      const r = await commands.getThroughput({ connectionId, range: "1h" })
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    refetchInterval: false,
  })
  const topology = useQuery({
    queryKey: ["topology", connectionId],
    queryFn: async () => {
      const r = await commands.getTopology({ connectionId })
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    refetchInterval: false,
  })
  const workers = useMemo(() => {
    const map = new Map<string, QueueWorker[]>()
    for (const g of groups)
      map.set(
        g.queue.queueRef.name,
        g.workers.map((w) => w.worker),
      )
    return map
  }, [groups])
  const model = useMemo(
    () =>
      buildMap({
        queues: queues ?? [],
        throughput: throughput.data,
        workers,
        topology: topology.data,
        onlyActive,
      }),
    [queues, throughput.data, workers, topology.data, onlyActive],
  )

  // View › Fit Map (⌘0).
  useCommandHandler("view.fitMap", () => setFit((f) => !f), {
    label: fit ? "Actual Size" : "Fit Map",
  })

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3" data-testid="map">
      <div className="flex flex-wrap items-center gap-3">
        <span className="flex items-center gap-2 text-sm">
          <Switch
            id="map-only-active"
            checked={onlyActive}
            onCheckedChange={(on) => onOnlyActiveChange(on)}
          />
          <label htmlFor="map-only-active">Only queues with activity</label>
        </span>
        {model.hidden > 0 ? (
          <span className="text-xs text-(--text2)">
            {model.hidden} idle {model.hidden === 1 ? "queue" : "queues"} hidden
          </span>
        ) : null}
        <Button
          size="sm"
          variant="ghost"
          className="gap-1.5"
          aria-pressed={fit}
          onClick={() => setFit((f) => !f)}
          data-testid="map-fit"
        >
          <ScanIcon className="size-3.5" aria-hidden="true" />
          {fit ? "Actual size" : "Fit"}
          <span className="font-tabular text-xs text-(--text2)">⌘0</span>
        </Button>
        <Legend />
      </div>
      {!queues ? (
        <p className="text-sm text-(--text2)">Reading the queues</p>
      ) : model.queues.length === 0 ? (
        <p className="text-sm text-(--text2)">
          {queues.length === 0
            ? "No queue on this connection."
            : "No queue had activity in the last hour. Turn off “Only queues with activity” to see them all."}
        </p>
      ) : (
        <div className="flex min-h-0 flex-1 gap-4">
          <MapCanvas
            model={model}
            fit={fit}
            selected={selected}
            onSelect={(s) => setSelected((cur) => (cur?.id === s.id ? null : s))}
          />
          {selected ? (
            <MapDetails
              connectionId={connectionId}
              model={model}
              selected={selected}
              onClose={() => setSelected(null)}
            />
          ) : null}
        </div>
      )}
    </div>
  )
}

function MapCanvas({
  model,
  fit,
  selected,
  onSelect,
}: {
  model: MapModel
  fit: boolean
  selected: Selected | null
  onSelect: (s: Selected) => void
}) {
  const viewport = useRef<HTMLDivElement>(null)
  const content = useRef<HTMLDivElement>(null)
  const nodes = useRef(new Map<string, HTMLElement>())
  const [anchors, setAnchors] = useState(new Map<string, Anchor>())
  const [size, setSize] = useState({ width: 0, height: 0 })
  const [scale, setScale] = useState(1)

  const measure = () => {
    const box = content.current
    if (!box) return
    const next = new Map<string, Anchor>()
    for (const [id, el] of nodes.current) {
      next.set(id, {
        left: el.offsetLeft,
        right: el.offsetLeft + el.offsetWidth,
        y: el.offsetTop + el.offsetHeight / 2,
      })
    }
    setAnchors(next)
    setSize({ width: box.scrollWidth, height: box.scrollHeight })
    const v = viewport.current
    // Fit shrinks the map to the window, never below what stays readable
    // (half size); a taller map then scrolls.
    setScale(
      fit && v ? Math.max(MIN_FIT, Math.min(1, v.clientHeight / Math.max(1, box.scrollHeight))) : 1,
    )
  }
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-measured when the map or the fit changes
  useLayoutEffect(measure, [model, fit])
  // biome-ignore lint/correctness/useExhaustiveDependencies: `measure` reads refs only
  useEffect(() => {
    if (typeof ResizeObserver === "undefined" || !viewport.current) return
    const observer = new ResizeObserver(() => measure())
    observer.observe(viewport.current)
    return () => observer.disconnect()
  }, [fit])

  const ref = (id: string) => (el: HTMLElement | null) => {
    if (el) nodes.current.set(id, el)
    else nodes.current.delete(id)
  }
  const maxWeight = Math.max(0, ...model.edges.map((e) => e.weight))

  return (
    <div ref={viewport} className="relative min-w-0 flex-1 overflow-auto" data-testid="map-canvas">
      <div style={fit ? { transform: `scale(${scale})`, transformOrigin: "top left" } : undefined}>
        <div
          ref={content}
          className="relative grid grid-cols-[minmax(11rem,15rem)_minmax(5rem,1fr)_minmax(11rem,15rem)] items-start pt-1 pb-4 pl-12"
        >
          <svg
            className="pointer-events-none absolute top-0 left-0"
            width={size.width}
            height={size.height}
            aria-hidden="true"
          >
            <defs>
              <marker
                id="map-flow-arrow"
                markerWidth="8"
                markerHeight="8"
                refX="7"
                refY="4"
                orient="auto"
              >
                <path d="M0,0 L8,4 L0,8 z" fill="var(--state-waiting)" />
              </marker>
            </defs>
            {model.flows.map((f) => {
              const a = anchors.get(`q:${f.parent}`)
              const b = anchors.get(`q:${f.child}`)
              if (!a || !b) return null
              const bend = Math.min(44, 14 + Math.abs(b.y - a.y) / 6)
              return (
                <path
                  key={`${f.parent}>${f.child}`}
                  d={`M ${a.left} ${a.y} C ${a.left - bend} ${a.y}, ${b.left - bend} ${b.y}, ${b.left - 2} ${b.y}`}
                  fill="none"
                  stroke="var(--state-waiting)"
                  strokeWidth={1.5}
                  markerEnd="url(#map-flow-arrow)"
                  data-testid="map-flow"
                />
              )
            })}
            {model.edges.map((e) => {
              const a = anchors.get(`q:${e.queue}`)
              const b = anchors.get(e.to === NO_WORKER ? "noWorker" : `p:${e.to}`)
              if (!a || !b) return null
              const dx = (b.left - a.right) / 2
              const width = maxWeight > 0 ? 1.5 + 7 * Math.sqrt(e.weight / maxWeight) : 1.5
              return (
                <path
                  key={`${e.queue}>${e.to}`}
                  d={`M ${a.right} ${a.y} C ${a.right + dx} ${a.y}, ${b.left - dx} ${b.y}, ${b.left} ${b.y}`}
                  fill="none"
                  stroke={TONE_STROKE[failureTone(e.failureRate)]}
                  strokeWidth={width}
                  strokeDasharray={e.dashed ? "5 4" : undefined}
                  opacity={0.85}
                  data-testid="map-edge"
                  data-dashed={e.dashed ? "true" : undefined}
                />
              )
            })}
          </svg>
          <ul className="flex flex-col gap-2" aria-label="Queues">
            {model.queues.map((q) => (
              <li key={q.name}>
                <QueueNode
                  queue={q}
                  innerRef={ref(`q:${q.name}`)}
                  selected={selected?.kind === "queue" && selected.id === q.name}
                  onClick={() => onSelect({ kind: "queue", id: q.name })}
                />
              </li>
            ))}
          </ul>
          <div />
          <ul className="flex flex-col gap-3" aria-label="Worker pools">
            {model.pools.map((p) => (
              <li key={p.id}>
                <button
                  ref={ref(`p:${p.id}`)}
                  type="button"
                  onClick={() => onSelect({ kind: "pool", id: p.id })}
                  className={nodeClass(selected?.kind === "pool" && selected.id === p.id)}
                  data-testid="map-pool"
                >
                  <span className="font-medium break-words">{p.name}</span>
                  <span className="text-xs text-(--text2)">
                    {p.workers} {p.workers === 1 ? "worker" : "workers"} · {p.queues.length}{" "}
                    {p.queues.length === 1 ? "queue" : "queues"}
                  </span>
                </button>
              </li>
            ))}
            {model.noWorker ? (
              <li>
                <button
                  ref={ref("noWorker")}
                  type="button"
                  onClick={() => onSelect({ kind: "noWorker", id: NO_WORKER })}
                  className={cn(
                    nodeClass(selected?.kind === "noWorker"),
                    "border-dashed border-(--state-delayed) text-(--state-delayed-text)",
                  )}
                  data-testid="map-no-worker"
                >
                  <span className="font-medium">No worker</span>
                  <span className="text-xs">Nobody takes these jobs</span>
                </button>
              </li>
            ) : null}
          </ul>
        </div>
      </div>
    </div>
  )
}

const nodeClass = (selected: boolean) =>
  cn(
    "relative z-10 flex w-full flex-col items-start gap-0.5 rounded-card border border-(--line) bg-(--surf) px-3 py-1.5 text-left text-sm hover:bg-(--surf2) focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)/60",
    selected && "border-(--accent) bg-(--sel)",
  )

function QueueNode({
  queue,
  innerRef,
  selected,
  onClick,
}: {
  queue: MapQueue
  innerRef: (el: HTMLElement | null) => void
  selected: boolean
  onClick: () => void
}) {
  const tone = failureTone(queue.failureRate)
  return (
    <button
      ref={innerRef}
      type="button"
      onClick={onClick}
      className={nodeClass(selected)}
      data-testid="map-queue"
    >
      <span className="flex w-full items-start gap-2">
        <span
          className="mt-1.5 size-2 shrink-0 rounded-full"
          style={{ background: TONE_STROKE[tone] }}
          aria-hidden="true"
        />
        <span className="min-w-0 font-medium break-words">{queue.name}</span>
      </span>
      <span className="pl-4 text-xs text-(--text2)">
        {rate(queue.outPerMin)}/min · {formatCount(queue.waiting)} waiting
        {queue.paused ? " · paused" : ""}
      </span>
    </button>
  )
}

function Legend() {
  return (
    <ul
      className="ml-auto flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-(--text2)"
      aria-label="Legend"
    >
      <li className="flex items-center gap-1.5">
        <svg width="28" height="10" aria-hidden="true">
          <line x1="0" y1="5" x2="28" y2="5" stroke="var(--text2)" strokeWidth="1.5" />
        </svg>
        <svg width="28" height="10" aria-hidden="true">
          <line x1="0" y1="5" x2="28" y2="5" stroke="var(--text2)" strokeWidth="6" />
        </svg>
        throughput
      </li>
      {(["ok", "warn", "danger"] as const).map((tone) => (
        <li key={tone} className="flex items-center gap-1.5">
          <span
            className="h-1 w-5 rounded"
            style={{ background: TONE_STROKE[tone] }}
            aria-hidden="true"
          />
          {tone === "ok" ? "under 2% failing" : tone === "warn" ? "2 to 10%" : "10% or more"}
        </li>
      ))}
      <li className="flex items-center gap-1.5">
        <svg width="28" height="10" aria-hidden="true">
          <line
            x1="0"
            y1="5"
            x2="28"
            y2="5"
            stroke="var(--text2)"
            strokeWidth="2"
            strokeDasharray="5 4"
          />
        </svg>
        paused or idle
      </li>
      <li className="flex items-center gap-1.5">
        <svg width="28" height="10" aria-hidden="true">
          <line x1="0" y1="5" x2="24" y2="5" stroke="var(--state-waiting)" strokeWidth="1.5" />
          <path d="M22,1 L28,5 L22,9 z" fill="var(--state-waiting)" />
        </svg>
        flow
      </li>
    </ul>
  )
}

function MapDetails({
  connectionId,
  model,
  selected,
  onClose,
}: {
  connectionId: string
  model: MapModel
  selected: Selected
  onClose: () => void
}) {
  return (
    <aside
      className="flex w-72 shrink-0 flex-col gap-3 rounded-card border border-(--line) p-3 text-sm"
      data-testid="map-details"
    >
      <div className="flex justify-end">
        <Button size="sm" variant="ghost" onClick={onClose} aria-label="Close">
          <XIcon className="size-3.5" aria-hidden="true" />
        </Button>
      </div>
      {selected.kind === "queue" ? (
        <QueueDetails
          connectionId={connectionId}
          queue={model.queues.find((q) => q.name === selected.id)}
        />
      ) : selected.kind === "pool" ? (
        (() => {
          const pool = model.pools.find((p) => p.id === selected.id)
          if (!pool) return null
          return (
            <>
              <h3 className="font-semibold break-words">{pool.name}</h3>
              <p className="text-(--text2)">
                {pool.workers} {pool.workers === 1 ? "worker" : "workers"} on{" "}
                {pool.addrs.length === 1 ? "one address" : `${pool.addrs.length} addresses`}
              </p>
              <ul className="flex flex-col gap-0.5 font-tabular text-xs break-all text-(--text2)">
                {pool.addrs.map((a) => (
                  <li key={a}>{a}</li>
                ))}
              </ul>
              <p className="text-xs text-(--text2)">Consumes {pool.queues.join(", ")}.</p>
              {pool.id.startsWith("queue:") ? (
                <p className="text-xs text-(--text2)" data-testid="map-unnamed-pool">
                  These workers have no name, so they are grouped by queue. Give them one (
                  <code className="font-mono">
                    {"new Worker(queue, fn, { name: 'billing-api' })"}
                  </code>
                  ) to see which process serves which queues.
                </p>
              ) : null}
            </>
          )
        })()
      ) : (
        <>
          <h3 className="font-semibold text-(--state-delayed-text)">No worker</h3>
          <p className="text-(--text2)">
            {model.edges
              .filter((e) => e.to === NO_WORKER)
              .map((e) => e.queue)
              .join(", ")}{" "}
            {model.edges.filter((e) => e.to === NO_WORKER).length === 1 ? "has" : "have"} no worker
            attached.
          </p>
        </>
      )}
    </aside>
  )
}

function QueueDetails({
  connectionId,
  queue,
}: {
  connectionId: string
  queue: MapQueue | undefined
}) {
  const causes = useQuery({
    queryKey: ["failureCauses", connectionId, queue?.name ?? ""],
    queryFn: async () => {
      const r = await commands.listFailureCauses({ connectionId, queue: queue?.name ?? null })
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    enabled: Boolean(queue) && (queue?.failureRate ?? 0) > 0,
  })
  if (!queue) return null
  const main = causes.data?.causes[0]
  const now = Date.now()
  return (
    <>
      <h3 className="font-semibold break-words">{queue.name}</h3>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1">
        <dt className="text-(--text2)">In / out</dt>
        <dd className="font-tabular">
          {queue.inPerMin === null ? "—" : `${queue.inCapped ? "≥ " : ""}${rate(queue.inPerMin)}`} /{" "}
          {rate(queue.outPerMin)} per minute
        </dd>
        <dt className="text-(--text2)">Failing</dt>
        <dd className="font-tabular">{(queue.failureRate * 100).toFixed(1)}%</dd>
        {main ? (
          <>
            <dt className="text-(--text2)">Main cause</dt>
            <dd className="font-tabular text-xs break-words">
              {main.signature || "No reason recorded"}
            </dd>
          </>
        ) : null}
        <dt className="text-(--text2)">Waiting</dt>
        <dd className="font-tabular">
          {formatCount(queue.waiting)}
          {queue.oldestWaitingMs !== null
            ? `, oldest ${formatAge(Math.max(now - queue.oldestWaitingMs, 0))} ago`
            : ""}
        </dd>
        <dt className="text-(--text2)">Workers</dt>
        <dd className="font-tabular">{queue.workers}</dd>
      </dl>
      <div className="flex flex-wrap gap-2">
        <Link
          to="/connections/$connectionId/queues/$queueName"
          params={{ connectionId, queueName: queue.name }}
          className="text-sm text-(--link) hover:underline"
        >
          Open queue
        </Link>
        <Link
          to="/connections/$connectionId/queues/$queueName"
          params={{ connectionId, queueName: queue.name }}
          search={{ view: "workers" } as never}
          className="text-sm text-(--link) hover:underline"
        >
          Show workers
        </Link>
      </div>
    </>
  )
}
