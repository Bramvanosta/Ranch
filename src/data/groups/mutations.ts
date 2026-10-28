import { useMutation, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"

import { useInvalidateHierarchy } from "@/data/hierarchy"
import { type CommandResult, unwrap } from "@/data/result"
import { toastFailure } from "@/data/toasts"
import { commands, type Group } from "@/lib/tauri/bindings"

import { groupKeys } from "./keys"

export function useCreateGroup() {
  const invalidate = useInvalidateHierarchy()
  return useMutation({
    mutationFn: async (name: string) => unwrap(await commands.createGroup({ name })),
    onSuccess: invalidate,
    onError: (err) => toastFailure("Could not create group", err),
  })
}

export function useRenameGroup() {
  const invalidate = useInvalidateHierarchy()
  return useMutation({
    mutationFn: async ({ id, name }: { id: string; name: string }) =>
      unwrap(await commands.renameGroup({ id, name })),
    onSuccess: invalidate,
    onError: (err) => toastFailure("Could not rename", err),
  })
}

/** `cascade` also removes the group's environments and connections. */
export function useDeleteGroup() {
  const invalidate = useInvalidateHierarchy()
  return useMutation({
    mutationFn: async ({ id, cascade }: { id: string; cascade: boolean }) => {
      unwrap(await commands.deleteGroup({ id, cascade }))
    },
    onSuccess: invalidate,
    onError: (err) => toastFailure("Could not delete group", err),
  })
}

/** Collapse state is persisted; the cache is patched optimistically so the chevron never lags. */
export function useSetGroupCollapsed() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, collapsed }: { id: string; collapsed: boolean }) => {
      unwrap(await commands.setGroupCollapsed({ id, collapsed }))
    },
    onMutate: async ({ id, collapsed }) => {
      await queryClient.cancelQueries({ queryKey: groupKeys.all })
      const previous = queryClient.getQueryData<Group[]>(groupKeys.all)
      queryClient.setQueryData<Group[]>(groupKeys.all, (old) =>
        old?.map((g) => (g.id === id ? { ...g, collapsed } : g)),
      )
      return { previous }
    },
    onError: (_err, _vars, ctx) => {
      if (ctx?.previous) queryClient.setQueryData(groupKeys.all, ctx.previous)
    },
  })
}

/** The sidebar's group order; the `groups` cache takes the new order at once. */
export function useReorderGroups() {
  const queryClient = useQueryClient()
  const invalidate = useInvalidateHierarchy()
  return useMutation({
    mutationFn: async (orderedIds: string[]) => {
      unwrap(await commands.reorderGroups({ orderedIds }))
    },
    onMutate: async (orderedIds) => {
      await queryClient.cancelQueries({ queryKey: groupKeys.all })
      const previous = queryClient.getQueryData<Group[]>(groupKeys.all)
      queryClient.setQueryData<Group[]>(groupKeys.all, (old) =>
        old ? sortGroupsByOrder(old, orderedIds) : [],
      )
      return { previous }
    },
    onError: (_err, _vars, ctx) => {
      if (ctx?.previous) queryClient.setQueryData(groupKeys.all, ctx.previous)
      toast.error("Could not reorder groups")
    },
    onSettled: invalidate,
  })
}

function sortGroupsByOrder(groups: Group[], groupOrder: string[]): Group[] {
  const rank = new Map<string, number>(groupOrder.map((id, i) => [id, i]))
  return [...groups].sort(
    (a, b) =>
      (rank.get(a.id) ?? Number.MAX_SAFE_INTEGER) - (rank.get(b.id) ?? Number.MAX_SAFE_INTEGER),
  )
}

/** A new group, for a flow that creates it with an environment. The caller refreshes the hierarchy. */
export function createGroup(name: string): Promise<CommandResult<Group>> {
  return commands.createGroup({ name })
}
