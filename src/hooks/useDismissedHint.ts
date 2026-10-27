import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useCallback } from "react"

import { commands } from "@/lib/tauri/bindings"

/**
 * Generic one-time-hint dismissal flag backed by
 * `kv_settings`.
 *
 * Key shape: `hint:{slug}:dismissed`. Value `"1"` means dismissed;
 * missing key (null from `getKvSetting`) means not yet dismissed.
 *
 * The first render returns `dismissed: undefined` (the kv read is in
 * flight). Callers should treat `undefined` as "don't show yet" to
 * avoid a flash on slow disks.
 *
 * Stays generic so future stories can register their own hints by
 * passing a different slug.
 */
export function useDismissedHint(slug: string) {
  const queryClient = useQueryClient()
  const key = `hint:${slug}:dismissed`

  const { data } = useQuery({
    queryKey: ["kv", key],
    queryFn: async () => {
      const r = await commands.getKvSetting(key)
      if (r.status === "error") throw new Error(r.error.message)
      return r.data === "1"
    },
    // Hint flags never auto-refresh — once read, the value is stable
    // for the session (we update the cache directly on dismiss).
    staleTime: Number.POSITIVE_INFINITY,
  })

  const mutation = useMutation({
    mutationFn: async () => {
      const r = await commands.setKvSetting(key, "1")
      if (r.status === "error") throw new Error(r.error.message)
    },
    onSuccess: () => {
      queryClient.setQueryData(["kv", key], true)
    },
  })

  // Stable `dismiss` reference so callers can include it in effect
  // deps without re-firing the effect every render. `mutation.mutate`
  // is itself stable per TanStack Query's contract.
  const mutate = mutation.mutate
  const dismiss = useCallback(() => {
    mutate()
  }, [mutate])

  return {
    dismissed: data,
    dismiss,
  }
}
