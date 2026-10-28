import { useQuery } from "@tanstack/react-query"
import { unwrap } from "@/data/result"
import { commands } from "@/lib/tauri/bindings"
import { updaterKeys } from "./keys"

/**
 * Whether a newer Ranch is published. Background-checked on app start, then
 * at most once a day. Offline or an unreachable feed is not worth a retry
 * storm: the next start asks again.
 */
export function useUpdateCheck() {
  return useQuery({
    queryKey: updaterKeys.check,
    queryFn: async () => {
      return unwrap(await commands.checkForUpdates())
    },
    staleTime: 24 * 60 * 60 * 1000,
    refetchOnWindowFocus: false,
    retry: false,
  })
}
