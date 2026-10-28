import type { AuditAction, AuditSource } from "@/lib/tauri/bindings"

/** Query keys of the audit log. */
export const auditKeys = {
  /** Every page of the audit log viewer, whatever its filter. */
  all: ["auditLog"] as const,
  page: (args: {
    from?: string
    to?: string
    connectionId?: string
    action?: AuditAction
    source?: AuditSource
    offset: number
    limit: number
  }) =>
    [
      ...auditKeys.all,
      args.from ?? "",
      args.to ?? "",
      args.connectionId ?? "",
      args.action ?? "",
      args.source ?? "",
      args.offset,
      args.limit,
    ] as const,
  /** What people and agents did on a connection since a moment (the Timeline's markers). */
  since: (connectionId: string, fromMs: number) => ["timelineAudit", connectionId, fromMs] as const,
  /** The scheduler pauses recorded on a connection. */
  schedulerPauses: (connectionId: string) => ["schedulerPausers", connectionId] as const,
}
