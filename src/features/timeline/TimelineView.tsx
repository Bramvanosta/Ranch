import { Link } from "@tanstack/react-router"
import { useEffect, useMemo, useRef, useState } from "react"
import { Segmented } from "@/components/data/Segmented"
import { Button } from "@/components/ui/button"
import { useAuditSince } from "@/data/audit/queries"
import { useThroughput } from "@/data/metrics/queries"
import { useWindowSummary } from "@/data/timeline/queries"
import { formatCount } from "@/domain/format"
import { ACTION_LABELS, whoOf } from "@/features/audit"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import { formatMoment } from "@/lib/time"
import { cn } from "@/lib/utils"
import {
  formatWait,
  fraction,
  latestMoment,
  type Marker,
  type Moment,
  placeMarkers,
  type QueueRow,
  queueRows,
  type Span,
  spanOf,
  TIMELINE_RANGES,
  type TimelineRange,
  tickLabel,
  ticks,
  timeAt,
  worstMoment,
} from "./timelineModel"

const MARKER_ROW_PX = 22

/**
 * The Timeline tab: what finished on the connection over time, every queue
 * together (completed under failed per bucket), what people (and, with AI
 * on, agents) did on a lane below, and the queues on one shared scale.
 *
 * It answers without being asked: the moment with the most failures is
 * selected when the tab opens and summarised beside the chart against the
 * rest of the range (the latest moment when nothing failed). Dragging across
 * the chart selects another moment.
 */
export function TimelineView({ connectionId }: { connectionId: string }) {
  const [range, setRange] = useState<TimelineRange>("24h")
  const aiOn = useAiEnabled()

  // The refresh setting (30 s floor: every queue on one axis).
  const throughput = useThroughput(connectionId, range, { floorMs: 30_000 })
  const t = throughput.data
  const span = t ? spanOf(t) : null

  const audit = useAuditSince(connectionId, span?.startMs ?? null)
  const markers: Marker[] = useMemo(
    () =>
      (audit.data ?? []).map((e) => {
        const agent = e.source === "agent"
        const label = ACTION_LABELS[e.action] ?? e.action
        return {
          key: e.id,
          atMs: Date.parse(e.createdAt),
          label: agent && aiOn ? `✦ ${label}` : label,
          kind: agent && aiOn ? "agent" : "audit",
          title: `${whoOf(e)} · ${label} · ${formatMoment(e.createdAt, { seconds: true })}`,
        }
      }),
    [audit.data, aiOn],
  )

  // A moment dragged by the user, else the one picked for them.
  const [picked, setPicked] = useState<{ a: number; b: number } | null>(null)
  const [dragging, setDragging] = useState(false)
  const worst = t ? worstMoment(t) : null
  const auto: Moment | null = span ? (worst ?? latestMoment(span)) : null
  const moment: Moment | null =
    picked && span
      ? {
          from: timeAt(span, Math.min(picked.a, picked.b)),
          to: timeAt(span, Math.max(picked.a, picked.b)),
        }
      : auto
  const why = picked
    ? "Your selection"
    : worst
      ? "The moment with the most failures"
      : "The latest moment"

  const { rows, scale } = t ? queueRows(t) : { rows: [], scale: 1 }
  const metricsOff = t?.metricsOffQueues ?? 0

  return (
    <div className="flex flex-col gap-4 xl:flex-row xl:items-start" data-testid="timeline">
      <div className="flex min-w-0 flex-1 flex-col gap-4">
        <div className="flex flex-wrap items-center gap-3">
          <Segmented
            label="Range"
            value={range}
            onChange={(r) => {
              setRange(r)
              setPicked(null)
            }}
            options={TIMELINE_RANGES.map((r) => ({ value: r, label: r }))}
          />
          <span className="ml-auto text-xs text-(--text2)">
            Drag across the chart to look at another moment.
          </span>
        </div>

        {throughput.isError ? (
          <p className="text-sm text-(--state-failed-text)">Could not read the queues' metrics.</p>
        ) : !t || !span || !moment ? (
          <p className="text-sm text-(--text2)">Reading the queues' metrics</p>
        ) : (
          <>
            <section
              className="flex flex-col gap-2 rounded-card border border-(--line) bg-(--surf) p-4"
              aria-label="Jobs finished, every queue"
            >
              <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                <h3 className="text-sm font-semibold">Jobs finished, every queue</h3>
                <span className="text-xs text-(--text2)">
                  <span className="text-(--state-completed-text)">■</span> completed{"  "}
                  <span className="text-(--state-failed-text)">■</span> failed
                </span>
              </div>
              <TotalsChart
                t={t}
                span={span}
                moment={moment}
                onPick={setPicked}
                onDragging={setDragging}
              />
              <Axis span={span} range={range} />
              <div className="border-t border-dashed border-(--line) pt-1">
                <span className="text-2xs text-(--text2)">
                  {aiOn ? "What people and agents did" : "What people did"}
                </span>
                <MarkerLane markers={markers} span={span} />
              </div>
            </section>

            <section
              className="flex flex-col rounded-card border border-(--line) p-4"
              aria-label="Queues"
              data-testid="timeline-queues"
            >
              <div className="flex flex-wrap items-baseline gap-x-3">
                <h3 className="text-sm font-semibold">Queues</h3>
                <span className="text-xs text-(--text2)">
                  on one scale, the most failures first
                </span>
              </div>
              {rows.length === 0 ? (
                <p className="py-3 text-sm text-(--text2)">No queue records BullMQ metrics here.</p>
              ) : (
                <ul className="mt-2">
                  {rows.map((row) => (
                    <QueueRowView
                      key={row.name}
                      row={row}
                      scale={scale}
                      connectionId={connectionId}
                    />
                  ))}
                </ul>
              )}
              {metricsOff > 0 ? (
                <p className="mt-3 text-xs text-(--text2)" data-testid="timeline-metrics-off">
                  {metricsOff === t.queues.length
                    ? "No queue"
                    : `${formatCount(metricsOff)} of ${formatCount(t.queues.length)} queues`}{" "}
                  {metricsOff === t.queues.length
                    ? "records"
                    : metricsOff === 1
                      ? "does not record"
                      : "do not record"}{" "}
                  BullMQ metrics, so the chart leaves {metricsOff === 1 ? "it" : "them"} out. Turn
                  on the worker option <code className="font-mono">metrics</code> to see them. The
                  summary beside reads the finished jobs themselves.
                </p>
              ) : null}
            </section>
          </>
        )}
      </div>

      {moment && !dragging ? (
        <WindowSummaryPanel
          connectionId={connectionId}
          window={moment}
          why={why}
          rangeFromMs={span?.startMs ?? moment.from}
          markers={markers}
          onReset={picked ? () => setPicked(null) : undefined}
        />
      ) : null}
    </div>
  )
}

/** Every queue together: completed under failed per bucket, with the moment shaded. */
function TotalsChart({
  t,
  span,
  moment,
  onPick,
  onDragging,
}: {
  t: { completed: number[]; failed: number[] }
  span: Span
  moment: Moment
  onPick: (sel: { a: number; b: number } | null) => void
  onDragging: (dragging: boolean) => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [drag, setDrag] = useState<{ a: number; b: number } | null>(null)
  const n = Math.max(1, t.completed.length)
  const peak = Math.max(1, ...t.completed.map((c, i) => c + (t.failed[i] ?? 0)))
  const a = drag ? Math.min(drag.a, drag.b) : fraction(span, moment.from)
  const b = drag ? Math.max(drag.a, drag.b) : fraction(span, moment.to)
  return (
    <div
      ref={ref}
      className="relative h-44 cursor-crosshair touch-none select-none"
      data-testid="timeline-chart"
      onPointerDown={(e) => {
        const f = fracIn(ref.current, e.clientX)
        if (f === null) return
        setDrag({ a: f, b: f })
        onDragging(true)
        e.currentTarget.setPointerCapture(e.pointerId)
      }}
      onPointerMove={(e) => {
        if (!drag) return
        const f = fracIn(ref.current, e.clientX, true)
        if (f !== null) setDrag({ ...drag, b: f })
      }}
      onPointerUp={() => {
        if (drag && Math.abs(drag.b - drag.a) >= 0.005) onPick(drag)
        setDrag(null)
        onDragging(false)
      }}
    >
      <div
        className="pointer-events-none absolute inset-y-0 rounded-sm border-x border-(--accent) bg-(--sel)"
        style={{ left: `${a * 100}%`, width: `${Math.max(0.4, (b - a) * 100)}%` }}
        data-testid="timeline-selection"
      />
      <svg
        viewBox={`0 0 ${n} 100`}
        preserveAspectRatio="none"
        className="relative block size-full"
        role="img"
        aria-label={`${formatCount(t.completed.reduce((x, y) => x + y, 0))} jobs completed and ${formatCount(t.failed.reduce((x, y) => x + y, 0))} failed in the range`}
      >
        {t.completed.map((done, i) => {
          const failed = t.failed[i] ?? 0
          const hDone = (done / peak) * 96
          const hFailed = (failed / peak) * 96
          return (
            // biome-ignore lint/suspicious/noArrayIndexKey: buckets are positional
            <g key={i}>
              {done > 0 ? (
                <rect
                  x={i + 0.12}
                  y={100 - hDone}
                  width={0.76}
                  height={hDone}
                  fill="var(--state-completed)"
                  opacity={0.7}
                />
              ) : null}
              {failed > 0 ? (
                <rect
                  x={i + 0.12}
                  y={100 - hDone - hFailed}
                  width={0.76}
                  height={hFailed}
                  fill="var(--state-failed)"
                />
              ) : null}
            </g>
          )
        })}
      </svg>
    </div>
  )
}

/** 0–1 along the chart; `null` outside it unless clamped. */
function fracIn(el: HTMLElement | null, clientX: number, clamp = false): number | null {
  if (!el) return null
  const rect = el.getBoundingClientRect()
  const f = (clientX - rect.left) / Math.max(1, rect.width)
  if (!clamp && (f < 0 || f > 1)) return null
  return Math.min(1, Math.max(0, f))
}

/** A queue: its sparkline on the shared scale, what it did in the range. */
function QueueRowView({
  row,
  scale,
  connectionId,
}: {
  row: QueueRow
  scale: number
  connectionId: string
}) {
  const n = Math.max(1, row.completed.length)
  const line = (values: number[]) =>
    values.map((v, i) => `${i + 0.5},${(30 - (v / scale) * 28).toFixed(2)}`).join(" ")
  return (
    <li
      className="grid grid-cols-[minmax(8rem,12rem)_minmax(0,1fr)_auto_auto] items-center gap-x-4 border-b border-(--line) py-1.5"
      data-testid="timeline-queue"
    >
      <Link
        to="/connections/$connectionId/queues/$queueId"
        params={{ connectionId, queueId: row.id }}
        className="text-sm break-words hover:underline"
      >
        {row.name}
      </Link>
      <svg
        viewBox={`0 0 ${n} 30`}
        preserveAspectRatio="none"
        className="block h-7 w-full"
        aria-hidden="true"
      >
        <polyline
          points={line(row.completed)}
          fill="none"
          stroke="var(--state-completed)"
          strokeWidth={1.5}
          vectorEffect="non-scaling-stroke"
        />
        <polyline
          points={line(row.failed)}
          fill="none"
          stroke="var(--state-failed)"
          strokeWidth={1.5}
          vectorEffect="non-scaling-stroke"
        />
      </svg>
      <span className="text-right font-tabular text-xs whitespace-nowrap">
        {formatCount(row.done)} done
      </span>
      <span
        className={cn(
          "text-right font-tabular text-xs whitespace-nowrap",
          row.failures > 0 ? "text-(--state-failed-text)" : "text-(--text2)",
        )}
      >
        {formatCount(row.failures)} failed
      </span>
    </li>
  )
}

function useWidth(ref: React.RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(800)
  useEffect(() => {
    const el = ref.current
    if (!el || typeof ResizeObserver === "undefined") return
    const observer = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width || 800))
    observer.observe(el)
    return () => observer.disconnect()
  }, [ref])
  return width
}

function MarkerLane({ markers, span }: { markers: Marker[]; span: Span }) {
  const ref = useRef<HTMLDivElement>(null)
  const width = useWidth(ref)
  const widthOf = (m: Marker) => (m.label.length * 6.5 + 18) / width
  const placed = placeMarkers(markers, span, widthOf)
  const rows = Math.max(2, ...placed.map((p) => p.row + 1))
  return (
    <div
      ref={ref}
      className="relative min-w-0 flex-1"
      style={{ height: rows * MARKER_ROW_PX + 6 }}
      data-testid="timeline-markers"
    >
      {placed.map((m) => (
        <span
          key={m.key}
          title={m.title}
          className={cn(
            "absolute flex items-center gap-1 text-2xs whitespace-nowrap",
            m.kind === "agent" ? "text-(--ai)" : "text-(--text2)",
          )}
          // Near the right edge the label reads leftwards from its dot.
          style={
            m.x + widthOf(m) > 1
              ? {
                  right: `${(1 - m.x) * 100}%`,
                  top: m.row * MARKER_ROW_PX + 4,
                  flexDirection: "row-reverse",
                }
              : { left: `${m.x * 100}%`, top: m.row * MARKER_ROW_PX + 4 }
          }
          data-testid={`timeline-marker-${m.kind}`}
        >
          <span
            className={cn(
              "size-2 shrink-0 rounded-full",
              m.kind === "agent" ? "bg-(--ai)" : "bg-(--text2)",
            )}
            aria-hidden="true"
          />
          {m.label}
        </span>
      ))}
    </div>
  )
}

function Axis({ span, range }: { span: Span; range: TimelineRange }) {
  return (
    <div className="flex">
      <div className="relative h-5 min-w-0 flex-1" aria-hidden="true">
        {ticks(span).map((t) => (
          <span
            key={t}
            className="absolute -translate-x-1/2 font-tabular text-2xs whitespace-nowrap text-(--text2)"
            style={{ left: `${fraction(span, t) * 100}%` }}
          >
            {tickLabel(t, range)}
          </span>
        ))}
      </div>
    </div>
  )
}

function WindowSummaryPanel({
  connectionId,
  window,
  why,
  rangeFromMs,
  markers,
  onReset,
}: {
  connectionId: string
  window: Moment
  why: string
  rangeFromMs: number
  markers: Marker[]
  /** Back to the moment picked automatically; absent when that is shown. */
  onReset?: () => void
}) {
  const summary = useWindowSummary(connectionId, window, rangeFromMs)
  const s = summary.data
  const when = (ms: number) =>
    new Date(ms).toLocaleString([], {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    })
  const queues = new Map<string, number>()
  for (const cause of s?.causes ?? []) {
    for (const job of cause.jobs) queues.set(job.queueName, (queues.get(job.queueName) ?? 0) + 1)
  }
  const around = markers.filter((m) => m.atMs >= window.from && m.atMs <= window.to)
  return (
    <aside
      className="flex flex-col gap-4 rounded-card border border-(--line) bg-(--surf) p-4 xl:sticky xl:top-0 xl:w-96 xl:shrink-0"
      aria-label="The selected moment"
      data-testid="timeline-summary"
    >
      <header className="flex flex-col gap-0.5">
        <span className="text-xs text-(--text2)" data-testid="timeline-why">
          {why}
        </span>
        <h3 className="font-semibold">
          {when(window.from)} to {when(window.to)}
        </h3>
        <span className="text-xs text-(--text2)">compared with the rest of the range</span>
        {onReset ? (
          <Button size="sm" variant="ghost" className="mt-1 self-start" onClick={onReset}>
            Back to the moment picked for you
          </Button>
        ) : null}
      </header>
      {summary.isError ? (
        <p className="text-sm text-(--state-failed-text)">
          Could not read the jobs of this moment.
        </p>
      ) : !s ? (
        <p className="text-sm text-(--text2)">Reading the jobs that finished then</p>
      ) : (
        <>
          <dl className="grid grid-cols-3 gap-2">
            <Stat
              label="Failed"
              value={formatCount(s.failed)}
              usual={`usually ${s.usualFailed.toFixed(1)}`}
              tone={s.failed > s.usualFailed * 1.5 && s.failed > 0 ? "danger" : undefined}
            />
            <Stat
              label="Done"
              value={formatCount(s.done)}
              usual={`usually ${s.usualDone.toFixed(1)}`}
            />
            <Stat
              label="Wait p95"
              value={formatWait(s.waitP95Ms)}
              usual={`usually ${formatWait(s.usualWaitP95Ms)}`}
              tone={
                s.waitP95Ms !== null &&
                s.usualWaitP95Ms !== null &&
                s.waitP95Ms > s.usualWaitP95Ms * 2
                  ? "danger"
                  : undefined
              }
            />
          </dl>
          {s.scanCapped ? (
            <p className="text-xs text-(--text2)">
              From the newest {formatCount(s.scanned)} finished jobs.
            </p>
          ) : null}
          <section className="flex flex-col gap-2" aria-label="What failed">
            <h4 className="text-sm font-semibold">What failed</h4>
            {s.causes.length > 0 ? (
              <ul className="flex flex-col gap-2">
                {s.causes.map((cause) => {
                  const queue = cause.jobs[0]?.queueId
                  return (
                    <li
                      key={cause.signature}
                      className="flex items-baseline gap-2"
                      data-testid="timeline-cause"
                    >
                      <span className="w-8 shrink-0 text-right font-tabular text-sm text-(--state-failed-text)">
                        {formatCount(cause.count)}
                      </span>
                      <span className="min-w-0 flex-1 font-tabular text-xs break-words">
                        {cause.signature || "No failure reason recorded"}
                      </span>
                      {queue ? (
                        <Link
                          to="/connections/$connectionId/queues/$queueId"
                          params={{ connectionId, queueId: queue }}
                          search={
                            { view: "jobs", state: "failed", group: "cause", page: 0 } as never
                          }
                          className="shrink-0 text-xs text-(--link) hover:underline"
                        >
                          Open
                        </Link>
                      ) : null}
                    </li>
                  )
                })}
              </ul>
            ) : (
              <p className="text-sm text-(--text2)">Nothing failed then.</p>
            )}
          </section>
          {queues.size > 0 ? (
            <section className="flex flex-col gap-1" aria-label="Where">
              <h4 className="text-sm font-semibold">Where</h4>
              <p className="text-sm">
                {[...queues.entries()]
                  .sort((x, y) => y[1] - x[1])
                  .map(([name, count]) => `${name} ${formatCount(count)}`)
                  .join(" · ")}
              </p>
            </section>
          ) : null}
          {around.length > 0 ? (
            <section className="flex flex-col gap-1" aria-label="Around it">
              <h4 className="text-sm font-semibold">Around it</h4>
              <ul className="flex flex-col gap-0.5 text-sm">
                {around.map((m) => (
                  <li key={m.key} className={m.kind === "agent" ? "text-(--ai)" : undefined}>
                    {m.title}
                  </li>
                ))}
              </ul>
            </section>
          ) : null}
          {s.failed > 0 ? (
            <Link
              to="/connections/$connectionId"
              params={{ connectionId }}
              search={{ view: "failed" } as never}
              className="mt-1 inline-flex h-control items-center justify-center rounded-control bg-(--accent) px-3 text-sm font-semibold text-(--accent-text)"
            >
              Open the failed jobs
            </Link>
          ) : null}
        </>
      )}
    </aside>
  )
}

function Stat({
  label,
  value,
  usual,
  tone,
}: {
  label: string
  value: string
  usual: string
  tone?: "danger"
}) {
  return (
    <div className="flex flex-col gap-0.5 rounded-card border border-(--line) px-3 py-2">
      <dt className="text-xs text-(--text2)">{label}</dt>
      <dd
        className={cn(
          "font-tabular text-lg font-semibold",
          tone === "danger" && "text-(--state-failed-text)",
        )}
      >
        {value}
      </dd>
      <dd className="text-xs text-(--text2)">{usual}</dd>
    </div>
  )
}
