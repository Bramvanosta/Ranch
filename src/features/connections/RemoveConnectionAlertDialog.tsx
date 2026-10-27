import { useMutation, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { toast } from "sonner"

import { ConfirmSheet } from "@/components/data/ConfirmSheet"
import type { Connection } from "@/lib/tauri/bindings"
import { commands } from "@/lib/tauri/bindings"
import { useSelectionStore } from "@/stores/useSelectionStore"

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
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const selectedId = useSelectionStore((s) => s.selectedConnectionId)
  const select = useSelectionStore((s) => s.select)

  const removeMutation = useMutation({
    mutationFn: async () => {
      const result = await commands.removeConnection(connection.id)
      if (result.status === "error") throw new Error(result.error.message)
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["connections"] })
      toast.success(`${connection.name} removed`)
      if (selectedId === connection.id) {
        select(null)
        navigate({ to: "/" })
      }
      onOpenChange(false)
    },
    onError: (err) => {
      toast.error(`Could not remove: ${err instanceof Error ? err.message : "unknown error"}`)
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
      onConfirm={() => removeMutation.mutate()}
      busy={removeMutation.isPending}
      testId="remove-connection-sheet"
    />
  )
}
