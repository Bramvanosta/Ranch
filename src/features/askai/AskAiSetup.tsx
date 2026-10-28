import { useEffect, useState } from "react"
import { toast } from "sonner"

import { Segmented } from "@/components/data/Segmented"
import { Input } from "@/components/ui/input"
import { useGroups } from "@/data/groups/queries"
import { ASK_AGENTS, type AskAgent } from "@/domain/askAgents"
import type { Group } from "@/lib/tauri/bindings"

import { useAskAiDefaultAgent, useCodeFolder, useSaveAskAi } from "./askAiSettings"

/**
 * Agents › Setup & safety › Ask AI: the agent ⇧⌥⌘A hands
 * things to, and per group the folder of its code (where Claude Code runs).
 */
export function AskAiSetup() {
  const agent = useAskAiDefaultAgent()
  const save = useSaveAskAi()
  const { data: groups } = useGroups()
  return (
    <section className="flex flex-col gap-4" data-testid="ask-ai-setup">
      <h2 className="text-lg font-semibold">Ask AI</h2>
      <div className="flex flex-col gap-1.5">
        <span className="text-sm font-medium">Default agent</span>
        <span className="text-xs text-(--text2)">
          What Agents › Ask AI About Selection (⇧⌥⌘A) uses. Every Ask AI menu offers the others.
        </span>
        <Segmented
          label="Default agent"
          value={agent}
          onChange={(v) => save.mutate({ agent: v as AskAgent })}
          options={ASK_AGENTS}
        />
      </div>
      <div className="flex flex-col gap-2">
        <span className="text-sm font-medium">Code folder per group</span>
        <span className="text-xs text-(--text2)">
          Claude Code starts in the folder of the group the job belongs to, so it can read the
          workers' code.
        </span>
        {(groups ?? []).length === 0 ? (
          <span className="text-sm text-(--text2)">No group yet.</span>
        ) : (
          <ul className="divide-y divide-(--line) rounded-card border border-(--line)">
            {(groups ?? []).map((g) => (
              <FolderRow key={g.id} group={g} />
            ))}
          </ul>
        )}
      </div>
    </section>
  )
}

function FolderRow({ group }: { group: Group }) {
  const folder = useCodeFolder(group.id)
  const save = useSaveAskAi()
  const [draft, setDraft] = useState("")
  useEffect(() => setDraft(folder ?? ""), [folder])
  const id = `ask-ai-folder-${group.id}`
  return (
    <li className="flex flex-wrap items-center gap-x-4 gap-y-1.5 px-3 py-2">
      <label htmlFor={id} className="min-w-32 text-sm break-words">
        {group.name}
      </label>
      <Input
        id={id}
        value={draft}
        placeholder="/Users/you/code/project"
        spellCheck={false}
        className="h-8 min-w-0 flex-1 font-mono text-xs"
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => {
          if (draft.trim() === (folder ?? "")) return
          save.mutate(
            { groupId: group.id, folder: draft },
            { onSuccess: () => toast.success(`Code folder of ${group.name} saved`) },
          )
        }}
      />
    </li>
  )
}
