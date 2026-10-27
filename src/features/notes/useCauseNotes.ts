import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import { useAiEnabled } from "@/lib/ai/aiFeatures"
import { type CauseNote, commands, type FailureCause } from "@/lib/tauri/bindings"

export const CAUSE_NOTES_QUERY_KEY = ["causeNotes"] as const
const NONE: CauseNote[] = []

/**
 * agents' resolution notes: one connection's, or every
 * one's. Nothing while AI features are off.
 */
export function useCauseNotes({
  connectionId = null,
  includeDismissed = false,
}: {
  connectionId?: string | null
  includeDismissed?: boolean
} = {}): CauseNote[] {
  const aiOn = useAiEnabled()
  const { data } = useQuery({
    queryKey: [...CAUSE_NOTES_QUERY_KEY, connectionId ?? "all", includeDismissed],
    queryFn: async () => {
      const r = await commands.listCauseNotes({ connectionId, includeDismissed })
      if (r.status === "error") throw new Error(r.error.message)
      return r.data
    },
    enabled: aiOn,
    refetchInterval: 10_000,
  })
  return aiOn ? (data ?? NONE) : NONE
}

/** The newest note on a cause (in `queueName`, or in any of its queues). */
export function noteForCause(
  notes: CauseNote[],
  cause: Pick<FailureCause, "signature" | "queueNames">,
  queueName?: string,
): CauseNote | undefined {
  return notes.find(
    (n) =>
      n.signature === cause.signature &&
      (queueName ? n.queueRef.name === queueName : cause.queueNames.includes(n.queueRef.name)),
  )
}

export function useDismissNote() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (noteId: string) => {
      const r = await commands.dismissCauseNote(noteId)
      if (r.status === "error") throw new Error(r.error.message)
    },
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: CAUSE_NOTES_QUERY_KEY }),
  })
}

/** The note as text: what to copy, and what a follow-up starts from. */
export function noteAsText(note: CauseNote): string {
  const steps = note.steps.map((s, i) => `${i + 1}. ${s}`).join("\n")
  return steps ? `${note.text}\n\n${steps}` : note.text
}

/** "Ask a follow-up": the note and its cause, ready to paste. */
export function followUpPrompt(note: CauseNote): string {
  const cause = note.signature === "" ? "a failure with no reason recorded" : note.signature
  return [
    `Earlier you left this note in Ranch on the failure cause "${cause}" of queue ${note.queueRef.name}${note.connectionName ? ` (connection ${note.connectionName})` : ""}:`,
    "",
    noteAsText(note),
    "",
    `Follow up: check whether it still fails, using the Ranch MCP tools (list_failure_causes, list_failed_jobs, inspect_job), and update the note with annotate_cause.`,
  ].join("\n")
}
