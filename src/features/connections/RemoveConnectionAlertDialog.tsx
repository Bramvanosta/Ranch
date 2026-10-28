import { useNavigate } from "@tanstack/react-router"
import { toast } from "sonner"

import { ConfirmSheet } from "@/components/data/ConfirmSheet"
import { useRemoveConnection } from "@/data/connections/mutations"
import { useSelectedConnectionId } from "@/hooks/useSelectedConnectionId"
import type { Connection } from "@/lib/tauri/bindings"

interface RemoveConnectionAlertDialogProps {
  connection: Connection
  open: boolean
  onOpenChange: (open: boolean) => void
}

/**
 * irreversible-action confirmation, as a sheet that
 * says what goes and what stays, Cancel focused, and on a production
 * connection the name typed first.
 *
 * On successful remove, if the removed connection was the currently-selected
 * one, navigate back to `/` so the canvas doesn't try to render a deleted id.
 */
export function RemoveConnectionAlertDialog({
  connection,
  open,
  onOpenChange,
}: RemoveConnectionAlertDialogProps) {
  const navigate = useNavigate()
  const selectedId = useSelectedConnectionId()

  const removeMutation = useRemoveConnection()
  const remove = () =>
    removeMutation.mutate(connection.id, {
      onSuccess: () => {
        toast.success(`${connection.name} removed`)
        if (selectedId === connection.id) {
          navigate({ to: "/" })
        }
        onOpenChange(false)
      },
    })

  // A production connection is protected: its name is typed before it goes.
  const isProtected = connection.envColor === "env-production"

  return (
    <ConfirmSheet
      open={open}
      onOpenChange={onOpenChange}
      title={`Remove “${connection.name}”?`}
      description="Ranch forgets this connection. Nothing changes on the Redis server."
      danger
      removed={[
        { label: "The connection, its settings and its saved password" },
        { label: "Its place in the sidebar and the home page" },
      ]}
      kept={[
        { label: "The Redis server, its queues and their jobs" },
        { label: "Past entries of the audit log" },
      ]}
      protectedName={isProtected ? connection.name : undefined}
      confirmLabel={removeMutation.isPending ? "Removing" : "Remove Connection"}
      onConfirm={() => remove()}
      busy={removeMutation.isPending}
      testId="remove-connection-sheet"
    />
  )
}
