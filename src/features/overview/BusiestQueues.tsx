import { Link } from "@tanstack/react-router"

import { HeatStrip } from "@/components/data/charts"
import { SectionHeader } from "@/components/data/SectionHeader"
import { formatCount } from "@/features/metrics/format"
import type { Throughput } from "@/lib/tauri/bindings"

const MAX = 6

/**
 * The queues that did the most over the last 6 hours,
 * each with its activity as a heat strip on the same axis.
 */
export function BusiestQueues({
  connectionId,
  throughput,
}: {
  connectionId: string
  throughput: Throughput | undefined
}) {
  const sum = (xs: number[]) => xs.reduce((a, b) => a + b, 0)
  const rows = (throughput?.queues ?? [])
    .map((q) => ({ ...q, done: sum(q.completed), lost: sum(q.failed) }))
    .sort(
      (a, b) =>
        Number(a.metricsOff) - Number(b.metricsOff) ||
        b.done + b.lost - (a.done + a.lost) ||
        a.queueRef.name.localeCompare(b.queueRef.name),
    )
    .slice(0, MAX)
  return (
    <section
      aria-label="Busiest queues"
      className="flex flex-col gap-2"
      data-testid="busiest-queues"
    >
      <SectionHeader
        title="Busiest queues"
        actions={<span className="text-xs text-(--text2)">last 6 hours</span>}
      />
      {rows.length === 0 ? (
        <p className="rounded-card border border-(--line) px-3 py-6 text-center text-sm text-(--text2)">
          {throughput ? "No queue on this connection." : "Counting"}
        </p>
      ) : (
        <ul className="divide-y divide-(--line) rounded-card border border-(--line)">
          {rows.map((q) => (
            <li
              key={q.queueRef.name}
              className="grid grid-cols-[minmax(6rem,10rem)_minmax(0,1fr)_auto] items-center gap-3 px-3 py-2 text-sm"
              data-testid="busiest-queue"
            >
              <Link
                to="/connections/$connectionId/queues/$queueName"
                params={{ connectionId, queueName: q.queueRef.name }}
                className="min-w-0 font-medium break-words hover:underline"
              >
                {q.queueRef.name}
              </Link>
              <HeatStrip
                completed={q.completed}
                failed={q.failed}
                metricsOff={q.metricsOff}
                label={q.queueRef.name}
              />
              <span className="text-right font-tabular text-xs whitespace-nowrap text-(--text2)">
                {q.metricsOff ? (
                  "—"
                ) : (
                  <>
                    {formatCount(q.done)} done
                    {q.lost > 0 ? (
                      <span className="text-(--state-failed-text)">
                        {" "}
                        · {formatCount(q.lost)} failed
                      </span>
                    ) : null}
                  </>
                )}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
