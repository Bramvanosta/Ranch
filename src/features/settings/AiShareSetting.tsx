import { useEffect, useState } from "react"

import { Textarea } from "@/components/ui/textarea"
import { aiShareMode, aiSharePatterns } from "@/data/settings/settings"
import { useSetting } from "@/data/settings/useSetting"
import type { ShareMode } from "@/domain/aiShare"
import { cn } from "@/lib/utils"

const MODES: Array<{ value: ShareMode; label: string; hint: string }> = [
  {
    value: "everything",
    label: "Everything",
    hint: "Payloads, return values and progress as they are.",
  },
  {
    value: "mask",
    label: "Mask fields whose name matches",
    hint: "Their values become •••, at any depth. The rest of the job is shared.",
  },
  {
    value: "names",
    label: "Names, states and errors only",
    hint: "No payload, return value, progress, template or log reaches an agent.",
  },
]

/**
 * Settings › AI features › "Job data shared with agents":
 * applied by `ranch-mcp` to every answer that carries job data. Mask is the
 * default.
 */
export function AiShareSetting() {
  const modeSetting = useSetting(aiShareMode)
  const patterns = useSetting(aiSharePatterns)
  const [draft, setDraft] = useState("")
  useEffect(() => {
    if (patterns.stored !== undefined) setDraft(patterns.stored)
  }, [patterns.stored])

  const mode = modeSetting.value
  return (
    <fieldset className="flex flex-col gap-3" data-testid="settings-ai-share">
      <legend className="mb-2 text-sm font-medium">Job data shared with agents</legend>
      {MODES.map((m) => (
        <label
          key={m.value}
          className={cn(
            "flex cursor-pointer items-start gap-2 rounded-card border border-(--line) p-3 text-sm",
            mode === m.value && "border-(--accent) bg-(--sel)",
          )}
        >
          <input
            type="radio"
            name="ai-share-mode"
            className="mt-1"
            checked={mode === m.value}
            onChange={() => modeSetting.set(m.value)}
          />
          <span className="flex min-w-0 flex-col gap-0.5">
            <span className="font-medium">{m.label}</span>
            <span className="text-xs text-(--text2)">{m.hint}</span>
          </span>
        </label>
      ))}
      {mode === "mask" ? (
        <div className="flex flex-col gap-1.5">
          <label htmlFor="ai-share-patterns" className="text-xs text-(--text2)">
            Field names to mask, separated by commas. A word masks every field that contains it
            (email masks customerEmail); * is a wildcard (card* masks cardNumber, not discard).
          </label>
          <Textarea
            id="ai-share-patterns"
            rows={2}
            value={draft}
            spellCheck={false}
            className="font-mono text-xs"
            onChange={(e) => setDraft(e.target.value)}
            onBlur={() => {
              if (patterns.stored !== undefined && draft.trim() !== patterns.stored) {
                patterns.set(draft.trim())
              }
            }}
          />
        </div>
      ) : null}
    </fieldset>
  )
}
