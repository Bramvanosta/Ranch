import { useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"

import { LOCK_STATE_QUERY_KEY } from "@/features/safety/useConnectionLockState"
import type { Connection } from "@/lib/tauri/bindings"

import { ConnectionSheet } from "./ConnectionSheet"

interface EditConnectionDialogProps {
  connection: Connection
  open: boolean
  onOpenChange: (open: boolean) => void
}

/**
 * Editing a connection: the same two-step sheet as a new
 * one, with the values filled and the passwords left blank (kept if so).
 */
export function EditConnectionDialog({
  connection,
  open,
  onOpenChange,
}: EditConnectionDialogProps) {
  const queryClient = useQueryClient()

  return (
    <ConnectionSheet
      mode="edit"
      connection={connection}
      open={open}
      onOpenChange={onOpenChange}
      onSaved={() => {
        // An edit can move the connection (and create the group /
        // environment it moves to), and respawns its actor.
        queryClient.invalidateQueries({ queryKey: ["connections"] })
        queryClient.invalidateQueries({ queryKey: ["groups"] })
        queryClient.invalidateQueries({ queryKey: ["environments"] })
        queryClient.invalidateQueries({ queryKey: LOCK_STATE_QUERY_KEY(connection.id) })
        toast.success(`${connection.name} updated`)
        onOpenChange(false)
      }}
    />
  )
}
