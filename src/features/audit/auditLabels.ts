import { agentLabel } from "@/features/mcp/agentLabel"
import type { AuditAction, AuditLogEntry } from "@/lib/tauri/bindings"

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
  removeScheduler: "Delete scheduler",
}

/** Who did it: "You", or the agent's readable name. */
export function whoOf(entry: Pick<AuditLogEntry, "source" | "agentClient">): string {
  return entry.source === "agent" ? (agentLabel(entry.agentClient) ?? "An agent") : "You"
}

/** "3 Oct 14:02:11" in the user's locale. */
export function formatWhen(iso: string): string {
  const d = new Date(iso)
  return Number.isNaN(d.getTime())
    ? iso
    : d.toLocaleString(undefined, {
        day: "numeric",
        month: "short",
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      })
}
