import { useMutation, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"

import { useInvalidateHierarchy } from "@/features/groups/useGroups"
import { LOCK_STATE_QUERY_KEY } from "@/features/safety/useConnectionLockState"
import { type Connection, commands, type Environment } from "@/lib/tauri/bindings"

/**
 * Move a connection to another environment. Shared by the
 * "Move to…" menu and drag-and-drop. The connection id does not change, so
 * the current selection and open tabs stay valid.
 */
export function useMoveConnection() {
  const queryClient = useQueryClient()
  const invalidateHierarchy = useInvalidateHierarchy()

  return useMutation({
    mutationFn: async (vars: { connectionId: string; environmentId: string; name: string }) => {
      const result = await commands.moveConnection({
        connectionId: vars.connectionId,
        environmentId: vars.environmentId,
      })
      if (result.status === "error") throw new Error(result.error.message)
      return result.data
    },
    onSuccess: (data, vars) => {
      invalidateHierarchy()
      if (data.newlyLocked) {
        void queryClient.invalidateQueries({ queryKey: LOCK_STATE_QUERY_KEY(vars.connectionId) })
        toast.info(`${vars.name} is now locked — it moved into a Production environment`)
      }
    },
    onError: (err) => {
      toast.error(`Could not move: ${err instanceof Error ? err.message : "unknown error"}`)
    },
  })
}

/**
 * Move a connection to another group, keeping its environment tag: into the
 * target group's environment of the same name, created there (same colour)
 * when it has none. Shared by the "Move to Group" menu and drag-and-drop.
 */
export function useMoveToGroup() {
  const move = useMoveConnection()
  return async (connection: Connection, groupId: string, environments: Environment[]) => {
    if (connection.groupId === groupId) return
    const label = connection.envLabel.trim().toLowerCase()
    let environmentId = environments.find(
      (e) => e.groupId === groupId && e.label.trim().toLowerCase() === label,
    )?.id
    if (!environmentId) {
      const created = await commands.createEnvironment({
        groupId,
        name: connection.envLabel,
        colorToken: connection.envColor,
      })
      if (created.status === "error") {
        toast.error(`Could not move: ${created.error.message}`)
        return
      }
      environmentId = created.data
    }
    move.mutate({ connectionId: connection.id, environmentId, name: connection.name })
  }
}
