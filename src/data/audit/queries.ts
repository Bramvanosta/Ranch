import { useQuery } from "@tanstack/react-query"
import { unwrap } from "@/data/result"
import { type AuditAction, type AuditSource, commands } from "@/lib/tauri/bindings"
import { auditKeys } from "./keys"

/**
 * TanStack Query hook for the audit log viewer.
 *
 * Page size defaults to 200 (the backend clamps `limit` to [1, 1000]).
 * Filters: date range (`from` / `to` as ISO-8601 strings), connection,
 * action, source. All optional: empty filter returns the full set.
 *
 * The log is **polled while the page is open**. An
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
    queryKey: auditKeys.page({ ...args, offset, limit }),
    queryFn: async () => {
      return unwrap(
        await commands.listAuditLogEntries({
          from: args.from ?? null,
          to: args.to ?? null,
          connectionId: args.connectionId ?? null,
          action: args.action ?? null,
          source: args.source ?? null,
          offset,
          limit,
        }),
      )
    },
    staleTime: 0,
    refetchInterval: AUDIT_POLL_MS,
    // A window that is not in front is not worth waking Redis-free SQLite
    // for either; the refetch on focus catches up.
    refetchIntervalInBackground: false,
  })
}

/**
 * The latest 200 entries of a connection since `fromMs`, newest first;
 * nothing until the moment is known (`null`).
 */
export function useAuditSince(connectionId: string, fromMs: number | null) {
  return useQuery({
    queryKey: auditKeys.since(connectionId, fromMs ?? 0),
    queryFn: async () => {
      return unwrap(
        await commands.listAuditLogEntries({
          from: new Date(fromMs ?? 0).toISOString(),
          to: null,
          connectionId,
          action: null,
          source: null,
          offset: 0,
          limit: 200,
        }),
      )
    },
    enabled: fromMs !== null,
  })
}

/** The latest 100 scheduler pauses recorded on a connection, newest first. */
export function useSchedulerPauses(connectionId: string) {
  return useQuery({
    queryKey: auditKeys.schedulerPauses(connectionId),
    queryFn: async () => {
      return unwrap(
        await commands.listAuditLogEntries({
          from: null,
          to: null,
          connectionId,
          action: "pauseScheduler",
          source: null,
          offset: 0,
          limit: 100,
        }),
      )
    },
    staleTime: 30_000,
  })
}
