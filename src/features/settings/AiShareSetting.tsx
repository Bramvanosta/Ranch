import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useEffect, useState } from "react"
import { toast } from "sonner"

import { Textarea } from "@/components/ui/textarea"
import { announceSettingsChange } from "@/lib/settingsSync"
import { commands } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

/** Mirrors `core/mcp/share.rs`. */
export const AI_SHARE_MODE_KEY = "ai_share_mode"
export const AI_SHARE_PATTERNS_KEY = "ai_share_patterns"
export const DEFAULT_SHARE_PATTERNS = "email, phone, token, password, secret, iban, card*"
export type ShareMode = "everything" | "mask" | "names"

const QUERY_KEY = ["kv", "aiShare"] as const

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
  const queryClient = useQueryClient()
  const { data } = useQuery({
    queryKey: QUERY_KEY,
    queryFn: async () => {
      const [mode, patterns] = await Promise.all([
        commands.getKvSetting(AI_SHARE_MODE_KEY),
        commands.getKvSetting(AI_SHARE_PATTERNS_KEY),
      ])
      const raw = mode.status === "ok" ? mode.data : null
      return {
        mode: (raw === "everything" || raw === "names" ? raw : "mask") as ShareMode,
        patterns: (patterns.status === "ok" ? patterns.data : null) ?? DEFAULT_SHARE_PATTERNS,
      }
    },
  })
  const [draft, setDraft] = useState("")
  useEffect(() => {
    if (data) setDraft(data.patterns)
  }, [data])

  const save = useMutation({
    mutationFn: async (next: { mode?: ShareMode; patterns?: string }) => {
      for (const [key, value] of [
        [AI_SHARE_MODE_KEY, next.mode],
        [AI_SHARE_PATTERNS_KEY, next.patterns],
      ] as const) {
        if (value === undefined) continue
        const r = await commands.setKvSetting(key, value)
        if (r.status === "error") throw new Error(r.error.message)
      }
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: QUERY_KEY })
      announceSettingsChange({ topic: "kv" })
    },
    onError: (e) => toast.error(e instanceof Error ? e.message : "Could not save"),
  })

  const mode = data?.mode ?? "mask"
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
            onChange={() => save.mutate({ mode: m.value })}
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
              if (data && draft.trim() !== data.patterns) save.mutate({ patterns: draft.trim() })
            }}
          />
        </div>
      ) : null}
    </fieldset>
  )
}
