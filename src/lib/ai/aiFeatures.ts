import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useEffect } from "react"
import { toast } from "sonner"
import { create } from "zustand"

import { hasAgents } from "@/lib/runtime/runtime"
import { announceSettingsChange } from "@/lib/settingsSync"
import { commands } from "@/lib/tauri/bindings"

/**
 * The AI features switch: kv `ai_enabled`, on unless set
 * to `false` (existing installs keep their agents). Off, everything about
 * agents leaves the interface — the Agents menu and sidebar item, ✦ lines,
 * approvals, agent filters — and `ranch-mcp` refuses every tool.
 *
 * `useAiEnabled()` is what components ask; `aiFeaturesOn()` the same for
 * code outside React (the menu model). Both are `hasAgents() && ai_enabled`:
 * the server image has no agents whatever the switch says.
 */
export const AI_ENABLED_KEY = "ai_enabled"
export const AI_ENABLED_QUERY_KEY = ["kv", AI_ENABLED_KEY] as const

interface AiState {
  enabled: boolean
  setEnabled: (enabled: boolean) => void
}

export const useAiStore = create<AiState>((set) => ({
  enabled: true,
  setEnabled: (enabled) => set({ enabled }),
}))

export function aiFeaturesOn(): boolean {
  return hasAgents() && useAiStore.getState().enabled
}

export function useAiEnabled(): boolean {
  const enabled = useAiStore((s) => s.enabled)
  return hasAgents() && enabled
}

/** Mounted once per window: reads the switch, and follows other windows. */
export function useAiEnabledSync(): void {
  const setEnabled = useAiStore((s) => s.setEnabled)
  const { data } = useQuery({
    queryKey: AI_ENABLED_QUERY_KEY,
    queryFn: async () => {
      const r = await commands.getKvSetting(AI_ENABLED_KEY)
      return r.status === "ok" ? r.data !== "false" : true
    },
    enabled: hasAgents(),
    staleTime: Number.POSITIVE_INFINITY,
  })
  useEffect(() => {
    if (data !== undefined) setEnabled(data)
  }, [data, setEnabled])
}

/** Settings › AI features: on or off, in every window at once. */
export function useSetAiFeatures() {
  const queryClient = useQueryClient()
  const setEnabled = useAiStore((s) => s.setEnabled)
  return useMutation({
    mutationFn: async (enabled: boolean) => {
      const r = await commands.setAiFeatures(enabled)
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    onSuccess: (enabled) => {
      queryClient.setQueryData(AI_ENABLED_QUERY_KEY, enabled)
      setEnabled(enabled)
      // Turning off denied the waiting requests.
      void queryClient.invalidateQueries({ queryKey: ["mcpPendingApprovals"] })
      announceSettingsChange({ topic: "kv" })
    },
    onError: (e) => toast.error(e instanceof Error ? e.message : "Could not change AI features"),
  })
}
