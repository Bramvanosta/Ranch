import { useMutation, useQueryClient } from "@tanstack/react-query"

import { askAiCodeFolder, askAiDefaultAgent } from "@/data/settings/settings"
import { settingQueryKey, useSetting, writeSetting } from "@/data/settings/useSetting"
import type { AskAgent } from "@/domain/askAgents"
import { announceSettingsChange } from "@/lib/settingsSync"

export function useAskAiDefaultAgent(): AskAgent {
  return useSetting(askAiDefaultAgent).value
}

/** The code folder of a group, `null` until set. */
export function useCodeFolder(groupId: string | null | undefined): string | null {
  const folder = useSetting(askAiCodeFolder(groupId ?? ""), { enabled: Boolean(groupId) })
  return groupId ? folder.value : null
}

export function useSaveAskAi() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (change: { agent: AskAgent } | { groupId: string; folder: string }) => {
      if ("agent" in change) {
        await writeSetting(askAiDefaultAgent, change.agent)
        return settingQueryKey(askAiDefaultAgent)
      }
      const folder = askAiCodeFolder(change.groupId)
      await writeSetting(folder, change.folder)
      return settingQueryKey(folder)
    },
    onSuccess: (queryKey) => {
      void queryClient.invalidateQueries({ queryKey })
      announceSettingsChange({ topic: "kv" })
    },
  })
}
