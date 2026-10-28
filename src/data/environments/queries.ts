import { useQuery } from "@tanstack/react-query"

import { unwrap } from "@/data/result"
import { commands } from "@/lib/tauri/bindings"

import { environmentKeys } from "./keys"

/**
 * Every environment, including the ones that hold no connection
 * yet (which `list_connections` cannot show). Feeds the placement fields of
 * `ConnectionForm` and the sidebar's "Move to…" menu.
 */
export function useEnvironments() {
  return useQuery({
    queryKey: environmentKeys.all,
    queryFn: async () => unwrap(await commands.listEnvironments()),
  })
}
