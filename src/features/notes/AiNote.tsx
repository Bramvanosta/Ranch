import { SparklesIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { AskAiMenu } from "@/features/askai/AskAiMenu"
import { useConnection } from "@/features/connections/useConnection"
import { formatAge } from "@/features/jobs/formatAge"
import { agentLabel } from "@/features/mcp/agentLabel"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"
import type { CauseNote } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import { followUpPrompt, noteAsText, useDismissNote } from "./useCauseNotes"

/**
 * An agent's note next to the failure cause it explains: the text, the steps, who wrote it and when, then Retry N, Ask
 * a follow-up, Copy and Dismiss.
 */
export function AiNote({
  note,
  retryCount,
  onRetry,
  className,
}: {
  note: CauseNote
  /** The jobs of the cause, for "Retry N". */
  retryCount?: number
  onRetry?: () => void
  className?: string
}) {
  const copy = useCopyToClipboard()
  const dismiss = useDismissNote()
  const connection = useConnection(note.connectionId)
  const who = note.clientHint ? (agentLabel(note.clientHint) ?? note.clientHint) : "Ranch"
  const age = formatAge(Math.max(Date.now() - Date.parse(note.createdAt), 0))
  return (
    <article
      className={cn(
        "flex flex-col gap-2 rounded-card border border-(--ai-line) bg-(--ai-bg) px-3 py-2 text-sm",
        className,
      )}
      data-testid="ai-note"
    >
      <p className="flex items-start gap-2">
        <SparklesIcon className="mt-0.5 size-4 shrink-0 text-(--ai)" aria-hidden="true" />
        <span className="min-w-0 break-words">{note.text}</span>
      </p>
      {note.steps.length > 0 ? (
        <ol className="ml-6 list-decimal text-sm marker:text-(--text2)">
          {note.steps.map((step, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: steps are ordered and may repeat
            <li key={i} className="break-words">
              {step}
            </li>
          ))}
        </ol>
      ) : null}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span className="mr-auto text-xs text-(--text2)" data-testid="ai-note-meta">
          Note from {who} · via Ranch MCP · {age} ago
        </span>
        {onRetry && retryCount ? (
          <Button size="sm" variant="default" onClick={onRetry}>
            Retry {retryCount.toLocaleString()}
          </Button>
        ) : null}
        {/* The follow-up goes to an agent like any Ask AI. */}
        <AskAiMenu
          target={{ kind: "prompt", connection: connection ?? null, prompt: followUpPrompt(note) }}
          label="Ask a follow-up"
          testId="ai-note-follow-up"
        />
        <Button size="sm" variant="ghost" onClick={() => void copy(noteAsText(note), "note")}>
          Copy
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={dismiss.isPending}
          onClick={() => dismiss.mutate(note.id)}
        >
          Dismiss
        </Button>
      </div>
    </article>
  )
}
