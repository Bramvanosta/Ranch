import { useQuery } from "@tanstack/react-query"

import { commands } from "@/lib/tauri/bindings"

export const ENVIRONMENTS_QUERY_KEY = ["environments"] as const

/**
 * Every environment, including the ones that hold no connection
 * yet (which `list_connections` cannot show). Feeds the placement fields of
 * `ConnectionForm` and the sidebar's "Move to…" menu.
 */
export function useEnvironments() {
  return useQuery({
    queryKey: ENVIRONMENTS_QUERY_KEY,
    queryFn: async () => {
      const result = await commands.listEnvironments()
      if (result.status === "error") throw new Error(result.error.message)
      return result.data
    },
  })
}
