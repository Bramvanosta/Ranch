import { useMutation, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"
import { connectionKeys } from "@/data/connections/keys"
import { setConnectionMcpReadonly } from "@/data/connections/mutations"
import { unwrap } from "@/data/result"
import { messageOf } from "@/data/toasts"
import { announceSettingsChange } from "@/lib/settingsSync"
import {
  type ActivityFilter,
  type ApprovalDecision,
  commands,
  type McpClient,
  type McpSafetySettings,
  type PendingApproval,
} from "@/lib/tauri/bindings"
import { mcpKeys } from "./keys"

export function useClearMcpActivity() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async () => {
      return unwrap(await commands.clearMcpActivity())
    },
    onSuccess: (removed) => {
      void queryClient.invalidateQueries({ queryKey: mcpKeys.activity })
      void queryClient.invalidateQueries({ queryKey: mcpKeys.sessions })
      toast.success(`Cleared ${removed} event${removed === 1 ? "" : "s"}`)
    },
    onError: (err) => toast.error(`Could not clear: ${messageOf(err)}`),
  })
}

export function useExportMcpActivity() {
  return useMutation({
    mutationFn: async (vars: { filter: ActivityFilter; format: "json" | "csv" }) => {
      const result = unwrap(await commands.exportMcpActivity(vars))
      return { content: result, format: vars.format }
    },
    onSuccess: ({ content, format }) => {
      // Same Blob + anchor pattern as the audit export; it works in Tauri's
      // webview without a filesystem-write capability.
      const filename = `ranch-mcp-activity-${new Date().toISOString().slice(0, 10)}.${format}`
      const blob = new Blob([content], {
        type: format === "json" ? "application/json" : "text/csv",
      })
      const url = URL.createObjectURL(blob)
      const a = document.createElement("a")
      a.href = url
      a.download = filename
      document.body.appendChild(a)
      a.click()
      document.body.removeChild(a)
      URL.revokeObjectURL(url)
      toast.success(`Exported ${filename}`)
    },
    onError: (err) => toast.error(`Export failed: ${messageOf(err)}`),
  })
}

/**
 * Approve or deny an agent request. Decided or `alreadyDecided` alike, it
 * leaves the pending list; a failure says the decision was not recorded.
 */
export function useDecideApproval() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (vars: {
      approvalId: string
      decision: ApprovalDecision
      reason?: string
    }) => {
      return unwrap(
        await commands.decidePendingApproval({
          approvalId: vars.approvalId,
          decision: vars.decision,
          reason: vars.reason?.trim() ? vars.reason.trim() : null,
        }),
      )
    },
    onSuccess: (_outcome, vars) => {
      queryClient.setQueryData<PendingApproval[]>(mcpKeys.pendingApprovals, (old) =>
        old?.filter((a) => a.id !== vars.approvalId),
      )
    },
    onError: (err) => toast.error(`Could not record your decision: ${messageOf(err)}`),
  })
}

export function useSetMcpSafetySettings() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (settings: McpSafetySettings) => {
      return unwrap(await commands.setMcpSafetySettings(settings))
    },
    onSuccess: (data) => {
      queryClient.setQueryData(mcpKeys.safety, data)
      announceSettingsChange({ topic: "mcp" })
    },
    onError: (err) => toast.error(`Could not save: ${messageOf(err)}`),
  })
}

/** `null` = follow the global default. */
export function useSetConnectionMcpReadonly() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (vars: { connectionId: string; mcpReadonly: boolean | null }) => {
      unwrap(await setConnectionMcpReadonly(vars))
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: connectionKeys.all })
      announceSettingsChange({ topic: "mcp" })
    },
    onError: (err) => toast.error(`Could not save: ${messageOf(err)}`),
  })
}

/**
 * Set Ranch up in an agent client. One generic command: the backend knows
 * whether the client is a JSON merge or goes through its own CLI. `label`
 * is the client's name, for the toasts.
 */
export function useInstallMcpClient() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({ client }: { client: McpClient; label: string }) => {
      return unwrap(await commands.installMcpForClient(client))
    },
    onSuccess: (_status, { label }) => {
      toast.success(`Installed Ranch MCP for ${label}.`, {
        description: `Restart ${label} to activate.`,
        duration: 6_000,
      })
      void queryClient.invalidateQueries({ queryKey: mcpKeys.installerStatus })
    },
    onError: (err) =>
      toast.error(`Install failed: ${messageOf(err)}`, {
        description: "Try the manual instructions below.",
        duration: 10_000,
      }),
  })
}
