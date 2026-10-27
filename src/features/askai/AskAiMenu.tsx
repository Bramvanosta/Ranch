import { SparklesIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { useAiEnabled } from "@/lib/ai/aiFeatures"

import { useCodeFolder } from "./askAiSettings"
import type { AskTarget } from "./buildPrompt"
import { useAskAi } from "./useAskAi"

/**
 * "Ask AI" on a job, a cause or a connection: Claude
 * Code (in Terminal, in the group's code folder), Claude Desktop, Cursor, or
 * the prompt on the clipboard. Nothing with AI features off.
 */
export function AskAiMenu({
  target,
  label = "Ask AI",
  size = "sm",
  testId = "ask-ai",
}: {
  target: AskTarget
  label?: string
  size?: "sm" | "default"
  testId?: string
}) {
  const aiOn = useAiEnabled()
  const folder = useCodeFolder(target.connection?.groupId)
  const ask = useAskAi()
  if (!aiOn) return null
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size={size} variant="ghost" className="gap-1.5" data-testid={testId}>
          <SparklesIcon className="size-3.5 text-(--ai)" aria-hidden="true" />
          {label}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-80">
        <DropdownMenuItem
          onSelect={() => void ask(target, "claudeCode", folder)}
          data-testid="ask-ai-claude-code"
        >
          <Choice
            title="Claude Code"
            hint={
              folder
                ? `Runs claude in Terminal, in ${folder}`
                : "Set the group's code folder in Agents › Setup & safety"
            }
          />
        </DropdownMenuItem>
        <DropdownMenuItem
          onSelect={() => void ask(target, "claudeDesktop", folder)}
          data-testid="ask-ai-claude-desktop"
        >
          <Choice title="Claude Desktop" hint="Copies the prompt and opens the app" />
        </DropdownMenuItem>
        <DropdownMenuItem
          onSelect={() => void ask(target, "cursor", folder)}
          data-testid="ask-ai-cursor"
        >
          <Choice title="Cursor" hint="Copies the prompt and opens the app" />
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onSelect={() => void ask(target, "copy", folder)}
          data-testid="ask-ai-copy"
        >
          <Choice title="Copy prompt" hint="To paste in any agent" />
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function Choice({ title, hint }: { title: string; hint: string }) {
  return (
    <span className="flex min-w-0 flex-col">
      <span>{title}</span>
      <span className="text-xs break-words text-(--text2)">{hint}</span>
    </span>
  )
}
