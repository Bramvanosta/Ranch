import { useQuery } from "@tanstack/react-query"

import { unwrap } from "@/data/result"
import { commands } from "@/lib/tauri/bindings"

import { groupKeys } from "./keys"

/**
 * Every group, including empty ones, in display order. The sidebar
 * uses this list as the backbone of its tree; `list_connections` alone cannot
 * show a group that has no connection yet.
 */
export function useGroups(options: { enabled?: boolean; staleTime?: number } = {}) {
  return useQuery({
    queryKey: groupKeys.all,
    queryFn: async () => unwrap(await commands.listGroups()),
    ...options,
  })
}
