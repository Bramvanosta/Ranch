import { useCallback } from "react"

import { hintDismissed } from "@/data/settings/settings"
import { useSetting } from "@/data/settings/useSetting"

/**
 * Whether a one-time hint was dismissed (`hint:{slug}:dismissed` in
 * `kv_settings`).
 *
 * The first render returns `dismissed: undefined` (the kv read is in
 * flight). Callers should treat `undefined` as "don't show yet" to
 * avoid a flash on slow disks.
 */
export function useDismissedHint(slug: string) {
  const { stored, set } = useSetting(hintDismissed(slug))
  // Stable, so callers can list it in effect deps.
  const dismiss = useCallback(() => set(true), [set])
  return { dismissed: stored, dismiss }
}
