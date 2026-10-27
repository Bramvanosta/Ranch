import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import { announceSettingsChange } from "@/lib/settingsSync"
import { commands } from "@/lib/tauri/bindings"

/** Who Ask AI hands things to by default, and where code lives. */
export type AskAgent = "claudeCode" | "claudeDesktop" | "cursor" | "copy"
export const ASK_AGENTS: Array<{ value: AskAgent; label: string }> = [
  { value: "claudeCode", label: "Claude Code" },
  { value: "claudeDesktop", label: "Claude Desktop" },
  { value: "cursor", label: "Cursor" },
  { value: "copy", label: "Copy prompt" },
]

const DEFAULT_AGENT_KEY = "ask_ai_default_agent"
const FOLDER_KEY = (groupId: string) => `ask_ai_folder:${groupId}`
const QUERY_KEY = ["kv", "askAi"] as const

export function useAskAiDefaultAgent(): AskAgent {
  const { data } = useQuery({
    queryKey: [...QUERY_KEY, "agent"],
    queryFn: async () => {
      const r = await commands.getKvSetting(DEFAULT_AGENT_KEY)
      const v = r.status === "ok" ? r.data : null
      return (ASK_AGENTS.some((a) => a.value === v) ? v : "claudeCode") as AskAgent
    },
  })
  return data ?? "claudeCode"
}

/** The code folder of a group, `null` until set. */
export function useCodeFolder(groupId: string | null | undefined): string | null {
  const { data } = useQuery({
    queryKey: [...QUERY_KEY, "folder", groupId ?? ""],
    queryFn: async () => {
      if (!groupId) return null
      const r = await commands.getKvSetting(FOLDER_KEY(groupId))
      const v = r.status === "ok" ? r.data?.trim() : null
      return v ? v : null
    },
    enabled: Boolean(groupId),
  })
  return data ?? null
}

export function useSaveAskAi() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (change: { agent: AskAgent } | { groupId: string; folder: string }) => {
      const [key, value] =
        "agent" in change
          ? [DEFAULT_AGENT_KEY, change.agent]
          : [FOLDER_KEY(change.groupId), change.folder.trim()]
      const r = await commands.setKvSetting(key, value)
      if (r.status === "error") throw new Error(r.error.message)
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: QUERY_KEY })
      announceSettingsChange({ topic: "kv" })
    },
  })
}
