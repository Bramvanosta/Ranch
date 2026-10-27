import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"

import { type ActivityFilter, commands } from "@/lib/tauri/bindings"

export const ACTIVITY_PAGE_SIZE = 100
const ACTIVITY_ENABLED_KEY = "mcp_activity_enabled"

export const EMPTY_FILTER: ActivityFilter = {
  from: null,
  to: null,
  clientHint: null,
  connectionId: null,
  kind: null,
  status: null,
  sessionId: null,
}

/**
 * What agents did. `ranch-mcp` is another process, so
 * there is no push: the panel refetches every few seconds while it is open.
 */
export function useMcpActivity(filter: ActivityFilter, page: number) {
  return useQuery({
    queryKey: ["mcpActivity", filter, page],
    queryFn: async () => {
      const result = await commands.listMcpActivity({
        filter,
        limit: ACTIVITY_PAGE_SIZE,
        offset: page * ACTIVITY_PAGE_SIZE,
      })
      if (result.status === "error") throw new Error(result.error.message)
      return result.data
    },
    refetchInterval: 5_000,
    placeholderData: (previous) => previous,
  })
}

export function useMcpActivitySessions(enabled: boolean) {
  return useQuery({
    queryKey: ["mcpActivitySessions"],
    queryFn: async () => {
      const result = await commands.listMcpActivitySessions()
      if (result.status === "error") throw new Error(result.error.message)
      return result.data
    },
    enabled,
    refetchInterval: 5_000,
  })
}

/** Recording switch — a plain kv setting (`true` unless explicitly "false"). */
export function useActivityEnabled() {
  const queryClient = useQueryClient()
  const query = useQuery({
    queryKey: ["mcpActivityEnabled"],
    queryFn: async () => {
      const result = await commands.getKvSetting(ACTIVITY_ENABLED_KEY)
      if (result.status === "error") throw new Error(result.error.message)
      return result.data !== "false"
    },
  })
  const mutation = useMutation({
    mutationFn: async (enabled: boolean) => {
      const result = await commands.setKvSetting(ACTIVITY_ENABLED_KEY, enabled ? "true" : "false")
      if (result.status === "error") throw new Error(result.error.message)
      return enabled
    },
    onSuccess: (enabled) => queryClient.setQueryData(["mcpActivityEnabled"], enabled),
    onError: (err) =>
      toast.error(`Could not save: ${err instanceof Error ? err.message : "unknown error"}`),
  })
  return { enabled: query.data ?? true, setEnabled: mutation.mutate }
}

export function useClearMcpActivity() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async () => {
      const result = await commands.clearMcpActivity()
      if (result.status === "error") throw new Error(result.error.message)
      return result.data
    },
    onSuccess: (removed) => {
      void queryClient.invalidateQueries({ queryKey: ["mcpActivity"] })
      void queryClient.invalidateQueries({ queryKey: ["mcpActivitySessions"] })
      toast.success(`Cleared ${removed} event${removed === 1 ? "" : "s"}`)
    },
    onError: (err) =>
      toast.error(`Could not clear: ${err instanceof Error ? err.message : "unknown error"}`),
  })
}

export function useExportMcpActivity() {
  return useMutation({
    mutationFn: async (vars: { filter: ActivityFilter; format: "json" | "csv" }) => {
      const result = await commands.exportMcpActivity(vars)
      if (result.status === "error") throw new Error(result.error.message)
      return { content: result.data, format: vars.format }
    },
    onSuccess: ({ content, format }) => {
      // Same Blob + anchor pattern as the audit export — works in Tauri's
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
    onError: (err) =>
      toast.error(`Export failed: ${err instanceof Error ? err.message : "unknown error"}`),
  })
}
