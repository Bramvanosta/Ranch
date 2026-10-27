import { useEffect } from "react"

/**
 * Minimal keyboard-shortcut hook.
 *
 * Pass a chord string like `"cmd+shift+s"` (case-insensitive). On
 * non-Mac platforms `cmd` resolves to `ctrl` automatically. The handler
 * is responsible for calling `e.preventDefault()` if it wants to suppress
 * the default behaviour.
 *
 * The handler is captured by reference each render via the closure over
 * `handler`; the effect's deps include `handler` so passing a fresh
 * reference is supported. For stable handlers, memoize at the call site.
 */
export function useKeyboardShortcut(chord: string, handler: (e: KeyboardEvent) => void) {
  useEffect(() => {
    const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform)
    const parts = chord
      .toLowerCase()
      .split("+")
      .map((s) => s.trim())
    const wantsMeta = parts.includes("cmd") || parts.includes("meta")
    const wantsCtrl = parts.includes("ctrl") || (wantsMeta && !isMac)
    const wantsShift = parts.includes("shift")
    const wantsAlt = parts.includes("alt") || parts.includes("option")
    const key = parts[parts.length - 1]

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() !== key) return
      // Modifier matrix — each desired modifier must be pressed, each
      // unwanted modifier must NOT be pressed (so cmd+s doesn't match
      // cmd+shift+s).
      const metaPressed = e.metaKey
      const ctrlPressed = e.ctrlKey
      if (isMac) {
        if (wantsMeta && !metaPressed) return
        if (!wantsMeta && metaPressed) return
        // On Mac we ignore ctrl unless the chord asked for it.
        if (wantsCtrl && !ctrlPressed) return
      } else {
        // On non-Mac, `cmd` in the chord maps to ctrl.
        if (wantsCtrl && !ctrlPressed) return
        if (!wantsCtrl && ctrlPressed) return
        if (metaPressed) return
      }
      if (wantsShift !== e.shiftKey) return
      if (wantsAlt !== e.altKey) return
      handler(e)
    }

    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [chord, handler])
}
