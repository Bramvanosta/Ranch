import { useNavigate } from "@tanstack/react-router"
import { useCallback } from "react"
import { toast } from "sonner"

import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import { commands } from "@/lib/tauri/bindings"

import type { AskAgent } from "./askAiSettings"
import { type AskTarget, buildPrompt } from "./buildPrompt"

const APP_NAME = { claudeDesktop: "Claude", cursor: "Cursor" } as const

/**
 * Hand a target to an agent: Claude Code runs in a terminal in
 * the group's code folder; Claude Desktop and Cursor get the prompt on the
 * clipboard and are opened; anything that cannot be automated falls back to
 * "copied".
 */
export function useAskAi() {
  const copy = useCopyToClipboard()
  const navigate = useNavigate()
  return useCallback(
    async (target: AskTarget, agent: AskAgent, folder: string | null) => {
      if (agent === "claudeCode" && !folder) {
        const group = target.connection?.groupName ?? "this group"
        toast.message(`Claude Code needs the code folder of ${group}`, {
          description:
            "Set it in Agents › Setup & safety › Ask AI; the prompt is copied meanwhile.",
          action: {
            label: "Set it",
            onClick: () => void navigate({ to: "/agents", search: { tab: "setup" } }),
          },
        })
        await copy(await buildPrompt(target), "prompt")
        return
      }
      let prompt: string
      try {
        prompt = await buildPrompt(target)
      } catch (e) {
        toast.error(e instanceof Error ? e.message : "Could not prepare the prompt")
        return
      }
      if (agent === "copy") {
        if (await copy(prompt, "prompt")) toast.success("Prompt copied")
        return
      }
      if (agent === "claudeCode") {
        const r = await commands.launchClaudeCode({ folder: folder as string, prompt })
        if (r.status === "ok") {
          toast.success("Claude Code is starting in Terminal")
        } else {
          await copy(prompt, "prompt")
          toast.error(r.error.message, { description: "The prompt is copied." })
        }
        return
      }
      await copy(prompt, "prompt")
      const r = await commands.openAgentApp(agent)
      if (r.status === "ok") {
        toast.success(`Prompt copied: paste it in ${APP_NAME[agent]}`)
      } else {
        toast.error(r.error.message)
      }
    },
    [copy, navigate],
  )
}
