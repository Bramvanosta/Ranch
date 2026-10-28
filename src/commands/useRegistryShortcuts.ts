import { useEffect } from "react"

import { matchesChord } from "./chords"
import { runCommand } from "./handlers"
import { COMMANDS } from "./registry"

/**
 * The registry's chords, for the whole window. On the desktop
 * the native menu runs them too (its accelerators); `runCommand` drops the
 * duplicate. A chord whose command does not apply now is left to the page.
 */
export function installRegistryShortcuts(): () => void {
  const withShortcut = COMMANDS.filter((c) => c.shortcut)
  const onKey = (e: KeyboardEvent) => {
    // ⌘⌫ deletes a line in a text field; it is not "Remove job" there.
    if (e.code === "Backspace" && isTypingTarget(e.target)) return
    for (const c of withShortcut) {
      if (c.shortcut && matchesChord(e, c.shortcut)) {
        if (runCommand(c.id, "key")) e.preventDefault()
        return
      }
    }
  }
  window.addEventListener("keydown", onKey)
  return () => window.removeEventListener("keydown", onKey)
}

export function useRegistryShortcuts() {
  useEffect(() => installRegistryShortcuts(), [])
}

function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  return target.matches("input, textarea, select") || target.isContentEditable
}
