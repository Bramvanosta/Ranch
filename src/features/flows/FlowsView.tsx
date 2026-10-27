import { useState } from "react"
import { StatePill } from "@/components/data/StatePill"
import { Skeleton } from "@/components/ui/skeleton"
import { formatAge } from "@/features/jobs/formatAge"
import type { FlowRoot, QueueRef } from "@/lib/tauri/bindings"

import { FlowGraph } from "./FlowGraph"
import { useFlow, useFlowRoots } from "./useFlow"

/**
 * Flows tab of a queue: the flow roots found in it, each
 * with what became of its children, expandable to the graph.
 *
 * Discovery is bounded — parents waiting for children, plus the most
 * recently finished jobs — and the view says so when the queue holds more.
 */
export function FlowsView({
  connectionId,
  queueRef,
}: {
  connectionId: string
  queueRef: QueueRef
}) {
  const { data, isLoading, isError, error } = useFlowRoots({ connectionId, queueRef })
  const [openId, setOpenId] = useState<string | null>(null)

  if (isLoading) {
    return (
      <div role="status" aria-label="Looking for flows" className="flex flex-col gap-2">
        {[0, 1, 2].map((i) => (
          <Skeleton key={i} className="h-10 w-full" />
        ))}
      </div>
    )
  }
  if (isError || !data) {
    return (
      <div
        role="alert"
        className="rounded-md border border-(--danger)/40 bg-(--danger)/10 p-3 text-sm text-(--state-failed-text)"
      >
        <p className="font-medium">Could not look for flows</p>
        <p className="text-xs opacity-80">
          {error instanceof Error ? error.message : "Unknown error"}
        </p>
      </div>
    )
  }

  const boundNote = data.capped
    ? `Looked at the parents waiting for children and the 250 most recent completed and failed jobs (${data.scanned.toLocaleString()} jobs) — older flows are not listed.`
    : null

  if (data.roots.length === 0) {
    return (
      <div
        data-testid="flows-empty"
        className="flex flex-col items-center gap-2 rounded-md border border-dashed border-(--line) bg-(--surf)/20 p-8 text-center"
      >
        <p className="text-sm font-medium">No flow starts in this queue.</p>
        <p className="max-w-prose text-xs text-(--text2)">
          Flows appear when jobs are created with BullMQ's{" "}
          <code className="font-mono">FlowProducer</code>: a parent job waits for its children,
          possibly in other queues. The flow is listed in the queue of its root job.
        </p>
        {boundNote ? <p className="text-2xs italic text-(--text2)">{boundNote}</p> : null}
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-2" data-testid="flows-view">
      <p className="text-xs text-(--text2)">
        {data.roots.length} {data.roots.length === 1 ? "flow" : "flows"}
        {boundNote ? ` · ${boundNote}` : ""}
      </p>
      <ul className="flex flex-col gap-2">
        {data.roots.map((root) => {
          const open = openId === root.jobRef.id
          return (
            <li key={root.jobRef.id} className="rounded-md border border-(--line)/40">
              <button
                type="button"
                onClick={() => setOpenId(open ? null : root.jobRef.id)}
                aria-expanded={open}
                data-testid="flow-root"
                className="flex w-full items-center gap-3 px-3 py-2 text-left text-sm hover:bg-(--surf)/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-(--accent)"
              >
                <span aria-hidden="true" className="w-3 text-(--text2)">
                  {open ? "▾" : "▸"}
                </span>
                <span className="min-w-0 flex-1 truncate">
                  <span className="font-medium">{root.name || "(unnamed)"}</span>
                  <span className="ml-2 font-mono text-2xs text-(--text2)">#{root.jobRef.id}</span>
                </span>
                <ChildrenSummary root={root} />
                <StatePill state={root.state} />
                <span className="w-10 shrink-0 text-right font-mono text-xs tabular-nums text-(--text2)">
                  {root.timestamp ? formatAge(Date.now() - root.timestamp) : "—"}
                </span>
              </button>
              {open ? (
                <div className="border-t border-(--line)/40 p-3">
                  <ExpandedFlow connectionId={connectionId} queueRef={queueRef} root={root} />
                </div>
              ) : null}
            </li>
          )
        })}
      </ul>
    </div>
  )
}

function ChildrenSummary({ root }: { root: FlowRoot }) {
  return (
    <span className="shrink-0 font-mono text-2xs tabular-nums text-(--text2)">
      {root.childrenCompleted}/{root.childrenTotal} done
      {root.childrenFailed > 0 ? (
        <span className="text-(--state-failed-text)"> · {root.childrenFailed} failed</span>
      ) : null}
      {root.childrenPending > 0 ? ` · ${root.childrenPending} pending` : ""}
    </span>
  )
}

function ExpandedFlow({
  connectionId,
  queueRef,
  root,
}: {
  connectionId: string
  queueRef: QueueRef
  root: FlowRoot
}) {
  const { data, isLoading, isError, error } = useFlow({
    connectionId,
    queueRef,
    jobId: root.jobRef.id,
  })
  if (isLoading) return <Skeleton className="h-32 w-full" />
  if (isError || !data) {
    return (
      <p role="alert" className="text-sm text-(--state-failed-text)">
        Could not read the flow: {error instanceof Error ? error.message : "unknown error"}
      </p>
    )
  }
  return <FlowGraph connectionId={connectionId} tree={data} highlightKey={null} />
}
