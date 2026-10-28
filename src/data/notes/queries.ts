import { useQuery } from "@tanstack/react-query"
import { unwrap } from "@/data/result"
import { useAiEnabled } from "@/lib/ai/aiFeatures"
import { type CauseNote, commands } from "@/lib/tauri/bindings"
import { noteKeys } from "./keys"

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
    queryKey: noteKeys.list(connectionId, includeDismissed),
    queryFn: async () => {
      return unwrap(await commands.listCauseNotes({ connectionId, includeDismissed }))
    },
    enabled: aiOn,
    refetchInterval: 10_000,
  })
  return aiOn ? (data ?? NONE) : NONE
}
