import { useNavigate } from "@tanstack/react-router"
import { useState } from "react"

import { useCapabilities } from "@/capabilities"
import { type MenuAction, separator } from "@/components/menus/TargetMenu"
import { usePauseQueue, useResumeQueue } from "@/data/queues/mutations"
import { useConnectionLockState } from "@/data/safety/queries"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import type { Queue } from "@/lib/tauri/bindings"
import { openRouteOrSay, queueRoute } from "@/lib/windows/openRoute"
import type { MaintenanceKind } from "./maintenance"
import { QueueMaintenanceDialog } from "./QueueMaintenanceDialog"

/**
 * What can be done to one queue from a right-click, wherever
 * the queue shows (sidebar row, Queues grid): open it, pause or resume it,
 * the maintenance actions, copy its key; the writes its backend declares.
 * They are greyed out while the connection is locked, like the queue's own
 * toolbar.
 */
export function useQueueMenu(connectionId: string, queue: Queue) {
  const navigate = useNavigate()
  const copy = useCopyToClipboard()
  const pause = usePauseQueue()
  const resume = useResumeQueue()
  const { data: lockState } = useConnectionLockState(connectionId)
  const locked = lockState === "locked"
  const [kind, setKind] = useState<MaintenanceKind | null>(null)
  const { can } = useCapabilities(connectionId)
  const queueId = queue.id
  const writes: MenuAction[] = [
    ...(can("queueControl")
      ? [
          {
            label: queue.isPaused ? "Resume" : "Pause",
            disabled: locked || pause.isPending || resume.isPending,
            run: () =>
              queue.isPaused
                ? resume.mutate({ connectionId, queueId })
                : pause.mutate({ connectionId, queueId }),
          },
        ]
      : []),
    // A protected connection asks for the queue's name in the sheet.
    ...(can("maintenance.retryAllFailed")
      ? [{ label: "Retry All Failed…", run: () => setKind("retryAllFailed") }]
      : []),
    ...(can("maintenance.clean") ? [{ label: "Clean…", run: () => setKind("clean") }] : []),
    ...(can("maintenance.drain")
      ? [{ label: "Drain…", danger: true, run: () => setKind("drain") }]
      : []),
  ]

  const actions = (): MenuAction[] => [
    {
      label: "Open",
      run: () =>
        void navigate({
          to: "/connections/$connectionId/queues/$queueId",
          params: { connectionId, queueId },
        }),
    },
    {
      label: "Open in New Tab",
      run: () => openRouteOrSay(queueRoute(connectionId, queueId), { tab: true }),
    },
    ...(writes.length > 0 ? [separator, ...writes] : []),
    separator,
    {
      label: "Copy Queue ID",
      run: () => copy(queueId, "Queue ID"),
    },
  ]

  const dialogs = (
    <QueueMaintenanceDialog
      kind={kind}
      onClose={() => setKind(null)}
      connectionId={connectionId}
      queueId={queueId}
    />
  )

  return { actions, dialogs }
}
