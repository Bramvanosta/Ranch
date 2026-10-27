import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import { commands, type Group } from "@/lib/tauri/bindings"

export const GROUPS_QUERY_KEY = ["groups"] as const

/** Longest accepted group name — mirrors `GROUP_NAME_MAX_CHARS` in `commands/groups.rs`. */
export const GROUP_NAME_MAX_CHARS = 64

/**
 * Every group, including empty ones, in display order. The sidebar
 * uses this list as the backbone of its tree; `list_connections` alone cannot
 * show a group that has no connection yet.
 */
export function useGroups() {
  return useQuery({
    queryKey: GROUPS_QUERY_KEY,
    queryFn: async () => {
      const result = await commands.listGroups()
      if (result.status === "error") throw new Error(result.error.message)
      return result.data
    },
  })
}

/**
 * `groups`, `environments` and `connections` are three views of one hierarchy
 * — refresh them together after anything that can create, move or delete.
 */
export function useInvalidateHierarchy() {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: GROUPS_QUERY_KEY })
    void queryClient.invalidateQueries({ queryKey: ["environments"] })
    void queryClient.invalidateQueries({ queryKey: ["connections"] })
  }
}

export function useCreateGroup() {
  const invalidate = useInvalidateHierarchy()
  return useMutation({
    mutationFn: async (name: string) => {
      const result = await commands.createGroup({ name })
      if (result.status === "error") throw new Error(result.error.message)
      return result.data
    },
    onSuccess: invalidate,
  })
}

export function useRenameGroup() {
  const invalidate = useInvalidateHierarchy()
  return useMutation({
    mutationFn: async ({ id, name }: { id: string; name: string }) => {
      const result = await commands.renameGroup({ id, name })
      if (result.status === "error") throw new Error(result.error.message)
      return result.data
    },
    onSuccess: invalidate,
  })
}

export function useDeleteGroup() {
  const invalidate = useInvalidateHierarchy()
  return useMutation({
    mutationFn: async ({ id, cascade }: { id: string; cascade: boolean }) => {
      const result = await commands.deleteGroup({ id, cascade })
      if (result.status === "error") throw new Error(result.error.message)
    },
    onSuccess: invalidate,
  })
}

/** Collapse state is persisted; the cache is patched optimistically so the chevron never lags. */
export function useSetGroupCollapsed() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, collapsed }: { id: string; collapsed: boolean }) => {
      const result = await commands.setGroupCollapsed({ id, collapsed })
      if (result.status === "error") throw new Error(result.error.message)
    },
    onMutate: async ({ id, collapsed }) => {
      await queryClient.cancelQueries({ queryKey: GROUPS_QUERY_KEY })
      const previous = queryClient.getQueryData<Group[]>(GROUPS_QUERY_KEY)
      queryClient.setQueryData<Group[]>(GROUPS_QUERY_KEY, (old) =>
        old?.map((g) => (g.id === id ? { ...g, collapsed } : g)),
      )
      return { previous }
    },
    onError: (_err, _vars, ctx) => {
      if (ctx?.previous) queryClient.setQueryData(GROUPS_QUERY_KEY, ctx.previous)
    },
  })
}

/**
 * Client-side mirror of the backend's name rules, so the inline input can
 * explain a rejection before the round-trip. The backend stays the authority.
 */
export function validateGroupName(
  raw: string,
  groups: Pick<Group, "id" | "name">[],
  excludeId?: string,
): string | null {
  const name = raw.trim()
  if (!name) return "Name cannot be empty"
  if ([...name].length > GROUP_NAME_MAX_CHARS) {
    return `Name cannot exceed ${GROUP_NAME_MAX_CHARS} characters`
  }
  const wanted = name.toLowerCase()
  if (groups.some((g) => g.id !== excludeId && g.name.toLowerCase() === wanted)) {
    return `A group named "${name}" already exists`
  }
  return null
}
