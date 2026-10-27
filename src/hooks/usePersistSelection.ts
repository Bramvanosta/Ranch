import { useEffect, useRef } from "react"
import { commands } from "@/lib/tauri/bindings"
import { isMainWindow } from "@/lib/windows/openRoute"
import { useSelectionStore } from "@/stores/useSelectionStore"

const KV_KEY = "selected_connection_id"

/**
 * `usePersistSelection` — write-through hook.
 *
 * Mount once at the route root (`__root.tsx`). Subscribes to the selection
 * store and writes every change back to `kv_settings.selected_connection_id`
 * so the next launch can restore it.
 *
 * Skips the very first effect run (initial mount) — at boot the store starts
 * at `null` and writing an empty string would clobber the persisted value
 * before the route layer has had a chance to navigate to the restored
 * connection. The `restored` flag flips after the first id mirror from the
 * route, after which writes flow normally.
 *
 * Empty-string is the "no selection" sentinel (kv_settings stores TEXT NOT NULL).
 */
export function usePersistSelection() {
  const id = useSelectionStore((s) => s.selectedConnectionId)
  const initialised = useRef(false)

  useEffect(() => {
    if (!initialised.current) {
      initialised.current = true
      return
    }
    // The next launch restores the main window's context only.
    if (!isMainWindow()) return
    void commands.setKvSetting(KV_KEY, id ?? "")
  }, [id])
}
