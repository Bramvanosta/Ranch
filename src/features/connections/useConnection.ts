import { useQuery } from "@tanstack/react-query"

import { type Connection, commands } from "@/lib/tauri/bindings"

/** One saved connection, from the shared `["connections"]` query. */
export function useConnection(connectionId: string | null | undefined): Connection | undefined {
  const { data } = useQuery({
    queryKey: ["connections"],
    queryFn: async () => {
      const r = await commands.listConnections()
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    enabled: Boolean(connectionId),
  })
  return data?.find((c) => c.id === connectionId)
}
