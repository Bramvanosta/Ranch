import { useMutation } from "@tanstack/react-query"
import { toast } from "sonner"
import { unwrap } from "@/data/result"
import { commands } from "@/lib/tauri/bindings"

/**
 * Download, verify and install the published update, then restart. On
 * success the app restarts and the mutation never resolves. A failure is
 * toasted, unless the caller shows it in place (`inline`).
 */
export function useInstallUpdate({ inline = false }: { inline?: boolean } = {}) {
  return useMutation({
    mutationFn: async () => {
      unwrap(await commands.installUpdate())
    },
    onError: (e) => {
      if (!inline) toast.error(e instanceof Error ? e.message : "The update failed")
    },
  })
}
