import { useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"

import { useInvalidateHierarchy } from "@/data/hierarchy"
import { safetyKeys } from "@/data/safety/keys"
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
  const invalidateHierarchy = useInvalidateHierarchy()

  return (
    <ConnectionSheet
      mode="edit"
      connection={connection}
      open={open}
      onOpenChange={onOpenChange}
      onSaved={() => {
        // An edit can move the connection (and create the group /
        // environment it moves to), and respawns its actor.
        invalidateHierarchy()
        queryClient.invalidateQueries({ queryKey: safetyKeys.lockState(connection.id) })
        toast.success(`${connection.name} updated`)
        onOpenChange(false)
      }}
    />
  )
}
