import { useQuery } from "@tanstack/react-query"

import { type AuditAction, type AuditSource, commands } from "@/lib/tauri/bindings"

/**
 * TanStack Query hook for the audit log viewer.
 *
 * Page size defaults to 200 (the backend clamps `limit` to [1, 1000]).
 * Filters: date range (`from` / `to` as ISO-8601 strings), connection,
 * action, source. All optional — empty filter returns the full set.
 *
 * The log is **polled while the page is open** (UI review 2026-09-21). An
 * agent's writes are inserted by the `ranch-mcp` process, not by this one:
 * there is no in-process mutation to invalidate on, and a row that appears
 * only on the next visit makes the page look broken while you watch an
 * agent work. `AUDIT_POLL_MS` is the same 5s cadence as the pending-approval
 * poller, and the query is `staleTime: 0` so coming back to the tab shows
 * what happened while it was closed.
 */
export const AUDIT_POLL_MS = 5_000
export const AUDIT_PAGE_SIZE = 200

export interface UseAuditLogArgs {
  from?: string
  to?: string
  connectionId?: string
  action?: AuditAction
  source?: AuditSource
  offset?: number
  limit?: number
}

export function useAuditLog(args: UseAuditLogArgs = {}) {
  const offset = args.offset ?? 0
  const limit = args.limit ?? AUDIT_PAGE_SIZE
  return useQuery({
    queryKey: [
      "auditLog",
      args.from ?? "",
      args.to ?? "",
      args.connectionId ?? "",
      args.action ?? "",
      args.source ?? "",
      offset,
      limit,
    ],
    queryFn: async () => {
      const result = await commands.listAuditLogEntries({
        from: args.from ?? null,
        to: args.to ?? null,
        connectionId: args.connectionId ?? null,
        action: args.action ?? null,
        source: args.source ?? null,
        offset,
        limit,
      })
      if (result.status === "error") {
        throw new Error(result.error.message)
      }
      return result.data
    },
    staleTime: 0,
    refetchInterval: AUDIT_POLL_MS,
    // A window that is not in front is not worth waking Redis-free SQLite
    // for either; the refetch on focus catches up.
    refetchIntervalInBackground: false,
  })
}
