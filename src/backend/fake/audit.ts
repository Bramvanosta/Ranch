import type { FakeHandlers } from "./handlers"
import { a, world } from "./world"

/** The audit log. */
export const auditHandlers = {
  // audit
  list_audit_log_entries: (args) => {
    const f = a(args)
    return world.audit
      .filter(
        (e) =>
          (!f.connectionId || e.connectionId === f.connectionId) &&
          (!f.action || e.action === f.action) &&
          (!f.source || e.source === f.source),
      )
      .slice(Number(f.offset ?? 0), Number(f.offset ?? 0) + Number(f.limit ?? 100))
  },
  export_audit_log: (args) => ({
    content: JSON.stringify(world.audit, null, 2),
    suggestedFilename: `ranch-audit.${String(a(args).format ?? "json")}`,
  }),
} satisfies FakeHandlers
