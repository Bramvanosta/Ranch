import { createFileRoute } from "@tanstack/react-router"

import { AuditLogPage } from "@/features/audit"

/**
 * top-level Audit route. Sibling of `/connections/*`;
 * The table and its detail column.
 */
export const Route = createFileRoute("/audit")({
  component: AuditLogPage,
})
