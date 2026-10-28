import { useQueries } from "@tanstack/react-query"
import { queueKeys } from "@/data/queues/keys"
import { fetchQueues } from "@/data/queues/queries"
import { failedTotal } from "@/domain/queueTriage"
import type { Connection } from "@/lib/tauri/bindings"

export type GroupSummary =
  | { kind: "empty" }
  | { kind: "offline" }
  | { kind: "count"; connections: number; failures: number }

/**
 * What a folded group says about itself: `empty`,
 * `offline` when no connection answers, otherwise how many connections it
 * holds and how many jobs failed in them. Reads the same `queueKeys.list(id)`
 * cache the rows poll, so it costs nothing while the group is open.
 */
export function useGroupSummary(connections: readonly Connection[]): GroupSummary {
  const results = useQueries({
    queries: connections.map((c) => ({
      queryKey: queueKeys.list(c.id),
      queryFn: () => fetchQueues(c.id),
      retry: false,
    })),
  })
  if (connections.length === 0) return { kind: "empty" }
  const settled = results.every((r) => r.isError || r.data)
  if (settled && results.every((r) => r.isError && !r.data)) return { kind: "offline" }
  const failures = results.reduce((sum, r) => sum + failedTotal(r.data), 0)
  return { kind: "count", connections: connections.length, failures }
}
