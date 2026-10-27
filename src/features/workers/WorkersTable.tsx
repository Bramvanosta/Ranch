import { useState } from "react"

import { Sheet } from "@/components/ui/sheet"
import { Skeleton } from "@/components/ui/skeleton"
import { formatAge } from "@/features/jobs/formatAge"
import type { QueueRef, QueueWorker } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import { useWorkers } from "./useWorkers"

/**
 * Workers tab of a queue: who is consuming it.
 *
 * BullMQ keeps no worker registry — a worker is a Redis connection named
 * after the queue — so the rows come from `CLIENT LIST`. Four states:
 *
 *   - workers → table (worker, address, connected for, idle for, last
 *     command); a row opens the raw client metadata;
 *   - none → "nothing is consuming this queue", and what makes one appear;
 *   - unsupported → the provider forbids `CLIENT LIST`: explained, with the
 *     server's own message — not shown as an error, and not as "0 workers";
 *   - error → the request itself failed.
 */
/** A worker blocked on an empty queue re-issues its pop every few seconds;
 *  far beyond that it is busy on a long job — or hung. */
const IDLE_ATTENTION_SECS = 60

interface WorkersTableProps {
  connectionId: string
  queueRef: QueueRef
}

export function WorkersTable({ connectionId, queueRef }: WorkersTableProps) {
  const { data, isLoading, isError, error } = useWorkers({ connectionId, queueRef })
  const [selected, setSelected] = useState<QueueWorker | null>(null)

  if (isLoading) {
    return (
      <div role="status" aria-label="Loading workers" className="flex flex-col gap-2 p-1">
        {[0, 1, 2].map((i) => (
          <Skeleton key={i} className="h-9 w-full" />
        ))}
      </div>
    )
  }

  if (isError || !data) {
    return (
      <div
        role="alert"
        className="flex flex-col gap-1 rounded-md border border-(--danger)/40 bg-(--danger)/10 p-3 text-sm text-(--state-failed-text)"
      >
        <p className="font-medium">Could not list workers</p>
        <p className="text-xs">{error instanceof Error ? error.message : "Unknown error"}</p>
      </div>
    )
  }

  if (!data.supported) {
    return (
      <EmptyState testId="workers-unsupported" title="This Redis does not allow listing clients">
        <p>
          BullMQ has no worker registry: workers are found with the Redis command{" "}
          <code className="font-mono">CLIENT LIST</code>, which this provider (or this user's ACL)
          forbids. Your workers may be running fine — Ranch just cannot see them here.
        </p>
        {data.unsupportedReason ? (
          <p className="font-mono text-2xs">{data.unsupportedReason}</p>
        ) : null}
      </EmptyState>
    )
  }

  // No refresh button of its own: the queue header's Refresh refetches
  // everything that belongs to the connection, this list included.
  const header = (
    <p className="text-xs text-(--text2)" data-testid="workers-count">
      {data.workers.length === 0
        ? "No worker attached"
        : `${data.workers.length} ${data.workers.length === 1 ? "worker" : "workers"} attached`}
    </p>
  )

  if (data.workers.length === 0) {
    return (
      <div className="flex flex-col gap-3">
        {header}
        <EmptyState testId="workers-empty" title="Nothing is consuming this queue">
          <p>
            Workers appear here as soon as a BullMQ <code className="font-mono">Worker</code> is
            connected to{" "}
            <code className="font-mono">
              {queueRef.prefix}:{queueRef.name}
            </code>{" "}
            on this Redis. Until then, waiting jobs stay waiting.
          </p>
        </EmptyState>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-3">
      {header}
      <div className="overflow-auto rounded-md border border-(--line)/40">
        <table className="w-full text-sm" data-testid="workers-table">
          <thead className="bg-(--surf)/30 text-left text-2xs font-semibold uppercase tracking-wide text-(--text2)">
            <tr>
              <th className="px-3 py-2">Worker</th>
              <th className="px-3 py-2">Address</th>
              <th className="px-3 py-2 text-right">Connected</th>
              <th className="px-3 py-2 text-right">Idle</th>
              <th className="px-3 py-2">Last command</th>
            </tr>
          </thead>
          <tbody>
            {data.workers.map((w) => (
              <tr
                key={w.id}
                tabIndex={0}
                onClick={() => setSelected(w)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault()
                    setSelected(w)
                  }
                }}
                data-testid="workers-row"
                className="cursor-pointer border-t border-(--line)/40 hover:bg-(--surf)/40 focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-(--accent)/60"
              >
                <td className="px-3 py-2">
                  {w.name ? (
                    <span className="font-medium">{w.name}</span>
                  ) : (
                    <span className="text-(--text2)">unnamed</span>
                  )}
                  <span className="ml-2 font-mono text-2xs text-(--text2)">#{w.id}</span>
                </td>
                <td className="px-3 py-2 font-mono text-xs">{w.addr}</td>
                <td className="px-3 py-2 text-right font-mono text-xs tabular-nums">
                  {formatAge(w.ageSecs * 1000)}
                </td>
                <td
                  className={cn(
                    "px-3 py-2 text-right font-mono text-xs tabular-nums",
                    w.idleSecs >= IDLE_ATTENTION_SECS && "text-(--state-delayed-text)",
                  )}
                  title={
                    w.idleSecs >= IDLE_ATTENTION_SECS
                      ? "No Redis command for a while: busy on a long job, or hung."
                      : undefined
                  }
                >
                  {formatAge(w.idleSecs * 1000)}
                </td>
                <td className="px-3 py-2 font-mono text-xs text-(--text2)">
                  {w.lastCommand ?? "—"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <Sheet
        open={selected !== null}
        onOpenChange={(open) => !open && setSelected(null)}
        testId="worker-detail"
        title={
          <>
            {selected?.name ?? "Unnamed worker"}{" "}
            <span className="font-mono text-sm text-(--text2)">#{selected?.id}</span>
          </>
        }
        description={
          <>
            Raw client metadata, as Redis reports it in{" "}
            <code className="font-mono">CLIENT LIST</code>
            {selected?.library ? `, ${selected.library}` : ""}.
          </>
        }
      >
        {selected ? (
          <dl className="grid max-h-80 grid-cols-[max-content_1fr] gap-x-4 gap-y-1 overflow-auto rounded-card border border-(--line) bg-(--surf) p-3 font-mono text-xs">
            {Object.entries(selected.raw).map(([key, value]) => (
              <div key={key} className="contents">
                <dt className="text-(--text2)">{key}</dt>
                <dd className="break-all">{value === "" ? "—" : value}</dd>
              </div>
            ))}
          </dl>
        ) : null}
      </Sheet>
    </div>
  )
}

function EmptyState({
  title,
  testId,
  children,
}: {
  title: string
  testId: string
  children: React.ReactNode
}) {
  return (
    <div
      data-testid={testId}
      className="flex flex-col items-center justify-center gap-2 rounded-md border border-dashed border-(--line) bg-(--surf)/20 p-8 text-center"
    >
      <p className="text-sm font-medium">{title}</p>
      <div className="flex max-w-prose flex-col gap-2 text-xs text-(--text2)">{children}</div>
    </div>
  )
}
