import { useNavigate } from "@tanstack/react-router"
import { useState } from "react"

import { type MenuAction, separator } from "@/components/menus/TargetMenu"
import { useConnectionLockState } from "@/features/safety/useConnectionLockState"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import type { Queue } from "@/lib/tauri/bindings"
import { openRouteOrSay, queueRoute } from "@/lib/windows/openRoute"

import type { MaintenanceKind } from "./maintenance"
import { QueueMaintenanceDialog } from "./QueueMaintenanceDialog"
import { usePauseQueue, useResumeQueue } from "./useQueueActions"

/**
 * What can be done to one queue from a right-click, wherever
 * the queue shows (sidebar row, Queues grid): open it, pause or resume it,
 * the maintenance actions, copy its key. Writes are greyed out while the
 * connection is locked, like the queue's own toolbar.
 */
export function useQueueMenu(connectionId: string, queue: Queue) {
  const navigate = useNavigate()
  const copy = useCopyToClipboard()
  const pause = usePauseQueue()
  const resume = useResumeQueue()
  const { data: lockState } = useConnectionLockState(connectionId)
  const locked = lockState === "locked"
  const [kind, setKind] = useState<MaintenanceKind | null>(null)
  const queueRef = queue.queueRef

  const actions = (): MenuAction[] => [
    {
      label: "Open",
      run: () =>
        void navigate({
          to: "/connections/$connectionId/queues/$queueName",
          params: { connectionId, queueName: queueRef.name },
        }),
    },
    {
      label: "Open in New Tab",
      run: () => openRouteOrSay(queueRoute(connectionId, queueRef.name), { tab: true }),
    },
    separator,
    {
      label: queue.isPaused ? "Resume" : "Pause",
      disabled: locked || pause.isPending || resume.isPending,
      run: () =>
        queue.isPaused
          ? resume.mutate({ connectionId, queueRef })
          : pause.mutate({ connectionId, queueRef }),
    },
    // A protected connection asks for the queue's name in the sheet.
    { label: "Retry All Failed…", run: () => setKind("retryAllFailed") },
    { label: "Clean…", run: () => setKind("clean") },
    { label: "Drain…", danger: true, run: () => setKind("drain") },
    separator,
    {
      label: "Copy Queue Key",
      run: () => copy(`${queueRef.prefix}:${queueRef.name}`, "Queue key"),
    },
  ]

  const dialogs = (
    <QueueMaintenanceDialog
      kind={kind}
      onClose={() => setKind(null)}
      connectionId={connectionId}
      queueRef={queueRef}
    />
  )

  return { actions, dialogs }
}
