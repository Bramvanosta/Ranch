import { ChevronDownIcon } from "lucide-react"
import { useState } from "react"

import { useCapabilities } from "@/capabilities"
import { useCommandHandler } from "@/commands/handlers"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import type { QueueId } from "@/lib/tauri/bindings"

import type { MaintenanceKind } from "./maintenance"
import { QueueMaintenanceDialog } from "./QueueMaintenanceDialog"

/**
 * queue-level maintenance: **Retry all failed**,
 * **Clean…**, **Drain…**, those the backend declares. Each opens the
 * preview-then-confirm dialog. On a locked connection the menu is disabled
 * with the reason; the backend refuses with `connectionLocked` regardless.
 */
interface QueueMaintenanceMenuProps {
  connectionId: string
  queueId: QueueId
  locked: boolean
}

export function QueueMaintenanceMenu({ connectionId, queueId, locked }: QueueMaintenanceMenuProps) {
  const [kind, setKind] = useState<MaintenanceKind | null>(null)
  const { can } = useCapabilities(connectionId)
  const retryAll = can("maintenance.retryAllFailed")
  const clean = can("maintenance.clean")
  const drain = can("maintenance.drain")
  // The same three actions in the Queue menu. On a
  // protected connection they open too: the sheet asks for the queue's name.
  useCommandHandler("queue.retryAllFailed", () => setKind("retryAllFailed"), { enabled: retryAll })
  useCommandHandler("queue.clean", () => setKind("clean"), { enabled: clean })
  useCommandHandler("queue.drain", () => setKind("drain"), { enabled: drain })

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
          {retryAll ? (
            <DropdownMenuItem
              onSelect={() => setKind("retryAllFailed")}
              data-testid="queue-maintenance-retry-all"
            >
              Retry All Failed…
            </DropdownMenuItem>
          ) : null}
          {retryAll && (clean || drain) ? <DropdownMenuSeparator /> : null}
          {clean ? (
            <DropdownMenuItem
              onSelect={() => setKind("clean")}
              data-testid="queue-maintenance-clean"
              className="text-(--state-failed-text) focus:text-(--state-failed-text)"
            >
              Clean…
            </DropdownMenuItem>
          ) : null}
          {drain ? (
            <DropdownMenuItem
              onSelect={() => setKind("drain")}
              data-testid="queue-maintenance-drain"
              className="text-(--state-failed-text) focus:text-(--state-failed-text)"
            >
              Drain…
            </DropdownMenuItem>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
      <QueueMaintenanceDialog
        kind={kind}
        onClose={() => setKind(null)}
        connectionId={connectionId}
        queueId={queueId}
      />
    </>
  )
}
