import type { ActivityFilter } from "@/lib/tauri/bindings"

/** Query keys of what agents did and asked, and of the MCP setup. */
export const mcpKeys = {
  /** Every page of the activity log, whatever its filter. */
  activity: ["mcpActivity"] as const,
  activityPage: (filter: ActivityFilter, page: number) =>
    [...mcpKeys.activity, filter, page] as const,
  sessions: ["mcpActivitySessions"] as const,
  /** The latest agent calls on one connection (its live events). */
  connectionActivity: (connectionId: string) => ["liveAgentActivity", connectionId] as const,
  pendingApprovals: ["mcpPendingApprovals"] as const,
  approvalHistory: ["mcpApprovalHistory"] as const,
  approvalRecord: (approvalId: string | null) => ["approvalRecord", approvalId] as const,
  safety: ["mcpSafetySettings"] as const,
  installerStatus: ["mcpInstallerStatus"] as const,
  manualSnippet: ["mcpManualSnippet"] as const,
}
