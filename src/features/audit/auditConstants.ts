import type { AuditAction } from "@/lib/tauri/bindings"

/**
 * List of every `AuditAction` variant in the same order
 * the filter dropdown displays them. Kept in a separate module so the
 * `AuditLogViewer` doesn't hardcode the action list inline (the
 * bindings.ts type is a union; we need an iterable at runtime).
 */
export const AUDIT_ACTIONS: ReadonlyArray<AuditAction> = [
  "retryJob",
  "removeJob",
  "promoteJob",
  "pauseQueue",
  "resumeQueue",
  "pauseScheduler",
  "resumeScheduler",
  "unlockConnection",
  "lockConnection",
  "addJob",
  "drainQueue",
  "cleanQueue",
  "retryAllFailed",
  "upsertScheduler",
  "removeScheduler",
]
