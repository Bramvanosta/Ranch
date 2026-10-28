import { useQueryClient } from "@tanstack/react-query"

import { connectionKeys } from "./connections/keys"
import { environmentKeys } from "./environments/keys"
import { groupKeys } from "./groups/keys"

/**
 * `groups`, `environments` and `connections` are three views of one hierarchy
 * Refresh them together after anything that can create, move or delete.
 */
export function useInvalidateHierarchy() {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: groupKeys.all })
    void queryClient.invalidateQueries({ queryKey: environmentKeys.all })
    void queryClient.invalidateQueries({ queryKey: connectionKeys.all })
  }
}
