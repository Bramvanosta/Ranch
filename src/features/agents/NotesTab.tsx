import { NotebookPenIcon } from "lucide-react"
import { useState } from "react"

import { EmptyState } from "@/components/data/EmptyState"
import { Switch } from "@/components/ui/switch"
import { useCauseNotes } from "@/data/notes/queries"
import { AiNote } from "@/features/notes"

/**
 * Agents › Notes: every resolution note with its cause and
 * where it is. Dismissed notes are hidden unless asked for.
 */
export function NotesTab() {
  const [showDismissed, setShowDismissed] = useState(false)
  const notes = useCauseNotes({ includeDismissed: showDismissed })
  return (
    <div className="flex flex-col gap-4" data-testid="agents-notes">
      <div className="flex items-center gap-2 text-sm">
        <Switch
          id="notes-show-dismissed"
          checked={showDismissed}
          onCheckedChange={setShowDismissed}
        />
        <label htmlFor="notes-show-dismissed">Show dismissed notes</label>
      </div>
      {notes.length === 0 ? (
        <EmptyState
          icon={<NotebookPenIcon />}
          title="No notes yet"
          description="When an agent explains a failure cause, its note shows here and next to the cause."
        />
      ) : (
        <ul className="flex flex-col gap-4">
          {notes.map((note) => (
            <li key={note.id} className="flex flex-col gap-1.5" data-testid="agents-note">
              <p className="flex flex-wrap items-baseline gap-x-2 text-sm">
                <span className="font-tabular break-words">
                  {note.signature === "" ? "No failure reason recorded" : note.signature}
                </span>
                <span className="text-xs text-(--text2)">
                  {note.queueName} · {note.connectionName ?? "a removed connection"}
                  {note.dismissedAt ? " · dismissed" : ""}
                </span>
              </p>
              <AiNote note={note} />
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
