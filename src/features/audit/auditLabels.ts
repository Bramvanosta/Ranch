import { agentLabel } from "@/features/mcp"
import type { AuditAction, AuditLogEntry } from "@/lib/tauri/bindings"
import { formatMoment } from "@/lib/time"

/** What an action is called in the audit log. */
export const ACTION_LABELS: Record<AuditAction, string> = {
  retryJob: "Retry job",
  removeJob: "Remove job",
  promoteJob: "Promote job",
  pauseQueue: "Pause queue",
  resumeQueue: "Resume queue",
  pauseScheduler: "Pause scheduler",
  resumeScheduler: "Resume scheduler",
  unlockConnection: "Unlock",
  lockConnection: "Lock",
  addJob: "Add job",
  drainQueue: "Drain queue",
  cleanQueue: "Clean queue",
  retryAllFailed: "Retry all failed",
  upsertScheduler: "Save scheduler",
  removeScheduler: "Remove scheduler",
}

/** Who did it: "You", or the agent's readable name. */
export function whoOf(entry: Pick<AuditLogEntry, "source" | "agentClient">): string {
  return entry.source === "agent" ? (agentLabel(entry.agentClient) ?? "An agent") : "You"
}

/** "Oct 3, 02:02:11 PM" in the user's locale. */
export function formatWhen(iso: string): string {
  return formatMoment(iso, { seconds: true })
}
