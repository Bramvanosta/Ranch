import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"

import { announceSettingsChange } from "@/lib/settingsSync"
import { commands, type McpSafetySettings } from "@/lib/tauri/bindings"

export const MCP_SAFETY_QUERY_KEY = ["mcpSafetySettings"] as const

/**
 * Global MCP safety settings. UI-only on purpose:
 * these commands are on the backend's MCP deny-list, an agent can never
 * reach them.
 */
export function useMcpSafetySettings() {
  return useQuery({
    queryKey: MCP_SAFETY_QUERY_KEY,
    queryFn: async () => {
      const result = await commands.getMcpSafetySettings()
      if (result.status === "error") throw new Error(result.error.message)
      return result.data
    },
  })
}

export function useSetMcpSafetySettings() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (settings: McpSafetySettings) => {
      const result = await commands.setMcpSafetySettings(settings)
      if (result.status === "error") throw new Error(result.error.message)
      return result.data
    },
    onSuccess: (data) => {
      queryClient.setQueryData(MCP_SAFETY_QUERY_KEY, data)
      announceSettingsChange({ topic: "mcp" })
    },
    onError: (err) =>
      toast.error(`Could not save: ${err instanceof Error ? err.message : "unknown error"}`),
  })
}

/** `null` = follow the global default. */
export function useSetConnectionMcpReadonly() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (vars: { connectionId: string; mcpReadonly: boolean | null }) => {
      const result = await commands.setConnectionMcpReadonly(vars)
      if (result.status === "error") throw new Error(result.error.message)
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["connections"] })
      announceSettingsChange({ topic: "mcp" })
    },
    onError: (err) =>
      toast.error(`Could not save: ${err instanceof Error ? err.message : "unknown error"}`),
  })
}

/** What the gate will apply: the connection's own flag, else the global default. */
export function effectiveMcpReadonly(
  connection: { mcpReadonly: boolean | null },
  readonlyDefault: boolean,
): boolean {
  return connection.mcpReadonly ?? readonlyDefault
}
