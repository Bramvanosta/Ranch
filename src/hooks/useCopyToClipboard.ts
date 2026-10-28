import { useCallback } from "react"
import { toast } from "sonner"

/**
 * Clipboard hook used by JobDrillDown's copy buttons.
 * Also used by JobList row context menus (and any other
 * UI surface that needs one-click copy).
 *
 * Uses `navigator.clipboard.writeText`: Tauri v2's webview supports
 * this without an extra plugin (no `tauri-plugin-clipboard-manager`
 * needed to copy text out). If the underlying call rejects (some
 * webview contexts deny writeText without user-gesture proximity), we
 * render a destructive toast rather than silently dropping.
 *
 * @returns A callback `(text, label) => Promise<boolean>` that performs
 *          the copy and surfaces a 2-second toast confirmation. The
 *          `label` is used in the toast text, e.g. `"job ID"`,
 *          `"payload"`, `"stack trace"`.
 */
export function useCopyToClipboard() {
  return useCallback(async (text: string, label: string): Promise<boolean> => {
    try {
      await navigator.clipboard.writeText(text)
      toast.success(`Copied ${label} to clipboard`, { duration: 2_000 })
      return true
    } catch {
      toast.error(`Couldn't copy ${label}. Try again.`, { duration: 3_000 })
      return false
    }
  }, [])
}
