import { useMutation, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"

import { createEnvironment } from "@/data/environments/mutations"
import { useInvalidateHierarchy } from "@/data/hierarchy"
import { type CommandResult, unwrap } from "@/data/result"
import { safetyKeys } from "@/data/safety/keys"
import { toastFailure } from "@/data/toasts"
import {
  type AddConnectionArgs,
  type Connection,
  commands,
  type Environment,
  type SetConnectionMcpReadonlyArgs,
  type UpdateConnectionArgs,
} from "@/lib/tauri/bindings"

import { connectionKeys } from "./keys"

/** Save a new connection. The caller refreshes the hierarchy once it is done. */
export function addConnection(args: AddConnectionArgs): Promise<CommandResult<Connection>> {
  return commands.addConnection(args)
}

/** Save an edited connection; its actor respawns with the new settings. */
export function updateConnection(args: UpdateConnectionArgs): Promise<CommandResult<Connection>> {
  return commands.updateConnection(args)
}

/** The agent read-only flag of one connection (`null`: the global default). */
export function setConnectionMcpReadonly(
  args: SetConnectionMcpReadonlyArgs,
): Promise<CommandResult<null>> {
  return commands.setConnectionMcpReadonly(args)
}

/** Fold / unfold a connection; the row follows at once. */
export function useSetConnectionCollapsed() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (vars: { connectionId: string; collapsed: boolean }) => {
      unwrap(await commands.setConnectionCollapsed(vars))
    },
    onMutate: async ({ connectionId, collapsed }) => {
      await queryClient.cancelQueries({ queryKey: connectionKeys.all })
      const previous = queryClient.getQueryData<Connection[]>(connectionKeys.all)
      queryClient.setQueryData<Connection[]>(connectionKeys.all, (old) =>
        old?.map((c) => (c.id === connectionId ? { ...c, sidebarCollapsed: collapsed } : c)),
      )
      return { previous }
    },
    onError: (_err, _vars, ctx) => {
      if (ctx?.previous) queryClient.setQueryData(connectionKeys.all, ctx.previous)
      toast.error("Could not save the folded state")
    },
  })
}

/** Remove a saved connection (and its keychain entries). */
export function useRemoveConnection() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (connectionId: string) => {
      unwrap(await commands.removeConnection(connectionId))
    },
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: connectionKeys.all }),
    onError: (err) => toastFailure("Could not remove", err),
  })
}

/** A copy of a connection, next to it, with " copy" in its name. */
export function useDuplicateConnection() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (connectionId: string) =>
      unwrap(await commands.duplicateConnection({ id: connectionId })),
    onSuccess: (newConnection) => {
      void queryClient.invalidateQueries({ queryKey: connectionKeys.all })
      toast.success(`Duplicated as ${newConnection.name}`)
    },
    onError: (err) => toastFailure("Could not duplicate", err),
  })
}

/**
 * Move a connection to another environment. Shared by the
 * "Move to…" menu and drag-and-drop. The connection id does not change, so
 * the current selection and open tabs stay valid.
 */
export function useMoveConnection() {
  const queryClient = useQueryClient()
  const invalidateHierarchy = useInvalidateHierarchy()

  return useMutation({
    mutationFn: async (vars: { connectionId: string; environmentId: string; name: string }) =>
      unwrap(
        await commands.moveConnection({
          connectionId: vars.connectionId,
          environmentId: vars.environmentId,
        }),
      ),
    onSuccess: (data, vars) => {
      invalidateHierarchy()
      if (data.newlyLocked) {
        void queryClient.invalidateQueries({ queryKey: safetyKeys.lockState(vars.connectionId) })
        toast.info(`${vars.name} is now locked: it moved into a Production environment`)
      }
    },
    onError: (err) => toastFailure("Could not move", err),
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
      const created = await createEnvironment({
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
