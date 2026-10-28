import { useNavigate } from "@tanstack/react-router"
import { useMemo } from "react"
import { useJobStates } from "@/capabilities"
import { StatePill } from "@/components/data/StatePill"
import { formatDurationMs } from "@/domain/format"
import { stateLabel } from "@/domain/jobStates"
import type { FlowTree } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import { layoutFlow, nodeSizeFor, summarizeFlow } from "./flowLayout"

/**
 * A flow as a top-to-bottom tree: every job with its
 * state, name, queue and duration; the job you came from is highlighted;
 * a click goes to that job, in whatever queue of the connection it lives.
 *
 * Layout is `flowLayout.ts` (a flow is a tree: no graph library); nodes are
 * real buttons, in reading order, over one SVG of connectors. Large flows
 * scroll.
 */
interface FlowGraphProps {
  connectionId: string
  tree: FlowTree
  /** Highlight another node than the tree's focus (the queue's Flows tab has none). */
  highlightKey?: string | null
}

export function FlowGraph({ connectionId, tree, highlightKey }: FlowGraphProps) {
  const states = useJobStates(connectionId)
  const navigate = useNavigate()
  const size = useMemo(() => nodeSizeFor(tree.nodes), [tree.nodes])
  const layout = useMemo(() => layoutFlow(tree.nodes, tree.rootKey, size), [tree, size])
  const summary = useMemo(() => summarizeFlow(tree.nodes), [tree.nodes])
  const focus = highlightKey === undefined ? tree.focusKey : highlightKey

  return (
    <div className="flex min-h-0 flex-col gap-2" data-testid="flow-graph">
      <p className="text-xs text-(--text2)" data-testid="flow-graph-summary">
        {summary.total} jobs · {summary.completed} completed
        {summary.failed > 0 ? (
          <span className="text-(--state-failed-text)"> · {summary.failed} failed</span>
        ) : null}
        {summary.active > 0 ? ` · ${summary.active} active` : ""}
        {summary.waitingChildren > 0 ? ` · ${summary.waitingChildren} waiting for children` : ""}
        {tree.truncated ? " · only the first 200 jobs are shown" : ""}
      </p>
      <div className="min-h-0 flex-1 overflow-auto rounded-md border border-(--line)/40 bg-(--surf)/10 p-4">
        <div className="relative" style={{ width: layout.width, height: layout.height }}>
          <svg
            width={layout.width}
            height={layout.height}
            className="pointer-events-none absolute inset-0 text-(--line)"
            aria-hidden="true"
          >
            {layout.edges.map((edge) => (
              <path
                key={`${edge.from}->${edge.to}`}
                d={edge.path}
                fill="none"
                stroke="currentColor"
                strokeWidth={1.5}
              />
            ))}
          </svg>
          {layout.nodes.map(({ node, x, y }) => {
            const isFocus = node.key === focus
            const label = stateLabel(node.state, states).toLowerCase()
            return (
              <button
                key={node.key}
                type="button"
                data-testid="flow-node"
                data-key={node.key}
                data-focus={isFocus ? "true" : undefined}
                aria-current={isFocus ? "true" : undefined}
                aria-label={`${node.name || "(unnamed)"}, ${label}, queue ${node.jobRef.queueName}, job ${node.jobRef.id}`}
                title={node.failedReason ?? undefined}
                disabled={node.missing}
                onClick={() =>
                  void navigate({
                    to: "/connections/$connectionId/queues/$queueId/$jobId",
                    params: {
                      connectionId,
                      queueId: node.jobRef.queueId,
                      jobId: node.jobRef.id,
                    },
                    search: { tab: "flow" },
                  })
                }
                style={{ left: x, top: y, width: size.width, height: size.height }}
                className={cn(
                  "absolute flex flex-col justify-center gap-1 rounded-md border border-(--line) bg-(--surf) px-3 text-left text-sm shadow-xs",
                  "hover:border-(--text)/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)",
                  isFocus && "border-(--accent) ring-2 ring-(--accent)/40",
                  node.state === "failed" && !isFocus && "border-[var(--state-failed)]/50",
                  node.missing && "cursor-not-allowed opacity-60",
                )}
              >
                {/* Line 1: the name and how long it took. Line 2: the state and
                    where the job lives: a long badge ("waiting children") next
                    to the name left room for five letters. */}
                <span className="flex items-baseline gap-2">
                  <span className="min-w-0 flex-1 font-medium leading-tight wrap-anywhere">
                    {node.missing ? "(removed)" : node.name || "(unnamed)"}
                  </span>
                  {node.durationMs !== null ? (
                    <span className="shrink-0 font-mono text-2xs tabular-nums text-(--text2)">
                      {formatDurationMs(node.durationMs)}
                    </span>
                  ) : null}
                </span>
                <span className="flex items-center gap-2 font-mono text-2xs text-(--text2)">
                  <StatePill state={node.state} states={states} className="shrink-0 font-sans" />
                  <span className="min-w-0 flex-1 wrap-anywhere">
                    {node.jobRef.queueName} #{node.jobRef.id}
                  </span>
                </span>
              </button>
            )
          })}
        </div>
      </div>
    </div>
  )
}
