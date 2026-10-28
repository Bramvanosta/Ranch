import { Link, useParams } from "@tanstack/react-router"
import { ChevronRightIcon } from "lucide-react"
import { useState } from "react"

import { useCapabilities } from "@/capabilities"
import { TargetMenu } from "@/components/menus/TargetMenu"
import { useQueueWorkerCounts } from "@/data/queues/queries"
import { type QueueProblem, triageQueues } from "@/domain/queueTriage"
import { useQueueMenu } from "@/features/queues"
import type { Queue } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

const PROBLEM_LABEL: Record<QueueProblem, (n: number) => string> = {
  failed: (n) => `${n} failed`,
  paused: () => "paused",
  noWorker: () => "no worker",
}

/**
 * The queues of an expanded connection: the ones that need the
 * user first, the others folded into "+ N more queues", expanded in
 * place, by name. Names wrap instead of being cut.
 */
export function SidebarQueues({
  connectionId,
  queues,
}: {
  connectionId: string
  queues: readonly Queue[] | undefined
}) {
  const [showRest, setShowRest] = useState(false)
  const { can, jobStates } = useCapabilities(connectionId)
  const counted = can("workers.count")
  const workers = useQueueWorkerCounts(connectionId, queues, counted)
  if (!queues) return null
  if (queues.length === 0) {
    return <p className="py-1 pl-9 pr-2 text-xs text-(--text2)">No BullMQ queue</p>
  }
  const { problems, rest } = triageQueues(queues, workers, jobStates)

  return (
    <ul className="flex flex-col" data-testid={`sidebar-queues-${connectionId}`}>
      {problems.map(({ queue, problem, count }) => (
        <QueueRow
          key={queue.name}
          connectionId={connectionId}
          queue={queue}
          badge={
            <span
              className={cn(
                "shrink-0 font-tabular text-2xs",
                problem === "failed" ? "text-(--state-failed-text)" : "text-(--state-delayed-text)",
              )}
            >
              {PROBLEM_LABEL[problem](count)}
            </span>
          }
        />
      ))}
      {rest.length > 0 ? (
        <li data-sidebar-item>
          <div data-sidebar-rowbox className="flex">
            <button
              type="button"
              data-sidebar-row
              data-sidebar-toggle
              aria-expanded={showRest}
              onClick={() => setShowRest((v) => !v)}
              className="flex min-h-row-queue flex-1 items-center gap-1.5 rounded-control py-1 pr-2 pl-9 text-left text-xs text-(--text2) outline-none hover:bg-(--surf2) hover:text-(--text) focus-visible:ring-2 focus-visible:ring-(--accent)/50"
            >
              <ChevronRightIcon
                aria-hidden="true"
                className={cn("size-3 shrink-0 transition-transform", showRest && "rotate-90")}
              />
              {problems.length > 0
                ? `+ ${rest.length} more ${rest.length === 1 ? "queue" : "queues"}`
                : `${rest.length} ${rest.length === 1 ? "queue" : "queues"}, all quiet`}
            </button>
          </div>
          {showRest ? (
            <ul className="flex flex-col">
              {rest.map((queue) => (
                <QueueRow key={queue.name} connectionId={connectionId} queue={queue} />
              ))}
            </ul>
          ) : null}
        </li>
      ) : null}
    </ul>
  )
}

function QueueRow({
  connectionId,
  queue,
  badge,
}: {
  connectionId: string
  queue: Queue
  badge?: React.ReactNode
}) {
  const params = useParams({ strict: false }) as { connectionId?: string; queueId?: string }
  const selected = params.connectionId === connectionId && params.queueId === queue.id
  // right-click: the queue's own menu, native on the desktop.
  const menu = useQueueMenu(connectionId, queue)
  return (
    <li data-sidebar-item>
      <TargetMenu actions={menu.actions} render={<div data-sidebar-rowbox className="flex" />}>
        <Link
          to="/connections/$connectionId/queues/$queueId"
          params={{ connectionId, queueId: queue.id }}
          data-sidebar-row
          aria-current={selected ? "page" : undefined}
          className={cn(
            "flex min-h-row-queue flex-1 items-center gap-2 rounded-control py-1 pr-2 pl-9 text-sm text-(--text) outline-none hover:bg-(--surf2) focus-visible:ring-2 focus-visible:ring-(--accent)/50",
            selected && "bg-(--sel) font-semibold",
          )}
        >
          <span className="min-w-0 flex-1 wrap-anywhere">{queue.name}</span>
          {badge}
        </Link>
      </TargetMenu>
      {menu.dialogs}
    </li>
  )
}
