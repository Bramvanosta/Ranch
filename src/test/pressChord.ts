import { isMacPlatform, parseChord } from "@/commands/chords"

/**
 * Press a registry chord ("cmd+shift+r") the way the platform under test
 * would send it: ⌘ is metaKey on a Mac, ctrlKey elsewhere. Dispatched on the
 * window, where the registry listens (`installRegistryShortcuts`).
 */
export function pressChord(chord: string, target: EventTarget = window) {
  const c = parseChord(chord)
  const mac = isMacPlatform()
  const code = /^[a-z]$/.test(c.key)
    ? `Key${c.key.toUpperCase()}`
    : /^[0-9]$/.test(c.key)
      ? `Digit${c.key}`
      : (({ backspace: "Backspace", comma: "Comma", enter: "Enter" } as Record<string, string>)[
          c.key
        ] ?? c.key)
  const event = new KeyboardEvent("keydown", {
    code,
    key: c.key === "backspace" ? "Backspace" : c.key,
    metaKey: mac && c.cmd,
    ctrlKey: mac ? c.ctrl : c.cmd || c.ctrl,
    altKey: c.alt,
    shiftKey: c.shift,
    bubbles: true,
    cancelable: true,
  })
  target.dispatchEvent(event)
  return event
}
