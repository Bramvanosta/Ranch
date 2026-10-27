import { ChevronDownIcon } from "lucide-react"
import { useState } from "react"

import { useCommandHandler } from "@/commands/handlers"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import type { QueueRef } from "@/lib/tauri/bindings"

import type { MaintenanceKind } from "./maintenance"
import { QueueMaintenanceDialog } from "./QueueMaintenanceDialog"

/**
 * queue-level maintenance: **Retry all failed**,
 * **Clean…**, **Drain…**. Each opens the preview-then-confirm dialog. On a
 * locked connection the menu is disabled with the reason; the backend
 * refuses with `connectionLocked` regardless.
 */
interface QueueMaintenanceMenuProps {
  connectionId: string
  queueRef: QueueRef
  locked: boolean
}

export function QueueMaintenanceMenu({
  connectionId,
  queueRef,
  locked,
}: QueueMaintenanceMenuProps) {
  const [kind, setKind] = useState<MaintenanceKind | null>(null)
  // The same three actions in the Queue menu. On a
  // protected connection they open too: the sheet asks for the queue's name.
  useCommandHandler("queue.retryAllFailed", () => setKind("retryAllFailed"))
  useCommandHandler("queue.clean", () => setKind("clean"))
  useCommandHandler("queue.drain", () => setKind("drain"))

  const trigger = (
    <Button
      type="button"
      size="sm"
      variant="default"
      data-testid="queue-maintenance-trigger"
      className="gap-1"
      title={locked ? "Protected: each operation asks for the queue's name" : undefined}
    >
      Maintenance
      <ChevronDownIcon className="size-3.5" aria-hidden="true" />
    </Button>
  )

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>{trigger}</DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-48">
          <DropdownMenuItem
            onSelect={() => setKind("retryAllFailed")}
            data-testid="queue-maintenance-retry-all"
          >
            Retry all failed…
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={() => setKind("clean")} data-testid="queue-maintenance-clean">
            Clean…
          </DropdownMenuItem>
          <DropdownMenuItem
            onSelect={() => setKind("drain")}
            data-testid="queue-maintenance-drain"
            className="text-(--state-failed-text) focus:text-(--state-failed-text)"
          >
            Drain…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <QueueMaintenanceDialog
        kind={kind}
        onClose={() => setKind(null)}
        connectionId={connectionId}
        queueRef={queueRef}
      />
    </>
  )
}
