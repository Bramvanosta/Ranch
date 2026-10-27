import { useState } from "react"

import { Input } from "@/components/ui/input"
import type { Group } from "@/lib/tauri/bindings"
import { cn } from "@/lib/utils"

import { validateGroupName } from "./useGroups"

interface GroupNameInputProps {
  initialValue?: string
  /** Existing groups, for the duplicate-name check. */
  groups: Pick<Group, "id" | "name">[]
  /** The group being renamed — excluded from the duplicate check. */
  excludeId?: string
  ariaLabel: string
  onSubmit: (name: string) => void
  onCancel: () => void
}

/**
 * Inline name field shared by "New group" and "Rename".
 * Enter submits, Escape cancels, blur submits when valid and cancels otherwise
 * (an invalid half-typed name must never trap focus in the sidebar).
 */
export function GroupNameInput({
  initialValue = "",
  groups,
  excludeId,
  ariaLabel,
  onSubmit,
  onCancel,
}: GroupNameInputProps) {
  const [draft, setDraft] = useState(initialValue)
  const [error, setError] = useState<string | null>(null)

  function submit(fromBlur: boolean) {
    const trimmed = draft.trim()
    if (trimmed === initialValue.trim()) {
      onCancel()
      return
    }
    const problem = validateGroupName(trimmed, groups, excludeId)
    if (problem) {
      if (fromBlur) onCancel()
      else setError(problem)
      return
    }
    onSubmit(trimmed)
  }

  return (
    <div className="flex min-w-0 flex-1 flex-col gap-0.5">
      <Input
        value={draft}
        onChange={(e) => {
          setDraft(e.target.value)
          if (error) setError(null)
        }}
        onKeyDown={(e) => {
          // Keep sidebar-level shortcuts (alt-arrow reorder, F2) out of the field.
          e.stopPropagation()
          if (e.key === "Enter") {
            e.preventDefault()
            submit(false)
          } else if (e.key === "Escape") {
            e.preventDefault()
            onCancel()
          }
        }}
        onBlur={() => submit(true)}
        onClick={(e) => e.stopPropagation()}
        autoFocus
        onFocus={(e) => e.currentTarget.select()}
        aria-label={ariaLabel}
        aria-invalid={error ? true : undefined}
        className={cn("h-6 px-1.5 py-0 text-xs", error && "border-(--danger)")}
      />
      {error ? (
        <p role="alert" className="px-1 text-2xs text-(--state-failed-text)">
          {error}
        </p>
      ) : null}
    </div>
  )
}
