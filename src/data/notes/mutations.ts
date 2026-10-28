import { useMutation, useQueryClient } from "@tanstack/react-query"
import { unwrap } from "@/data/result"
import { commands } from "@/lib/tauri/bindings"
import { noteKeys } from "./keys"

/** Hide a note; it stays listed among the dismissed ones. */
export function useDismissNote() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (noteId: string) => {
      unwrap(await commands.dismissCauseNote(noteId))
    },
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: noteKeys.all }),
  })
}
