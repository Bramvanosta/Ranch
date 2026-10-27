/**
 * Keyboard chords of the command registry: one notation,
 * "cmd+shift+r", read three ways — to match a key event, to build a native
 * menu accelerator, and to show the hint (⇧⌘R on a Mac, Ctrl+Shift+R
 * elsewhere). `cmd` is ⌘ on a Mac and Ctrl elsewhere, like the rest of Ranch.
 */

export const isMacPlatform = () =>
  typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform)

interface Chord {
  cmd: boolean
  ctrl: boolean
  alt: boolean
  shift: boolean
  key: string
}

export function parseChord(chord: string): Chord {
  const parts = chord.toLowerCase().split("+")
  const key = parts[parts.length - 1] ?? ""
  return {
    cmd: parts.includes("cmd"),
    ctrl: parts.includes("ctrl"),
    alt: parts.includes("alt"),
    shift: parts.includes("shift"),
    key,
  }
}

/** `KeyboardEvent.code` of a chord key: layout- and Option-proof (⌥P is "π"). */
function codeOf(key: string): string {
  if (/^[a-z]$/.test(key)) return `Key${key.toUpperCase()}`
  if (/^[0-9]$/.test(key)) return `Digit${key}`
  return { comma: "Comma", backspace: "Backspace", enter: "Enter", period: "Period" }[key] ?? key
}

export function matchesChord(e: KeyboardEvent, chord: string, mac = isMacPlatform()): boolean {
  const c = parseChord(chord)
  const wantMeta = mac && c.cmd
  const wantCtrl = mac ? c.ctrl : c.cmd || c.ctrl
  return (
    e.code === codeOf(c.key) &&
    e.metaKey === wantMeta &&
    e.ctrlKey === wantCtrl &&
    e.altKey === c.alt &&
    e.shiftKey === c.shift
  )
}

const ACCEL_KEY: Record<string, string> = {
  comma: ",",
  backspace: "Backspace",
  enter: "Enter",
  period: ".",
}

/** A Tauri / muda accelerator, e.g. "CmdOrCtrl+Shift+R". */
export function toAccelerator(chord: string, mac = isMacPlatform()): string {
  const c = parseChord(chord)
  const parts: string[] = []
  if (c.ctrl && (mac || !c.cmd)) parts.push("Ctrl")
  if (c.alt) parts.push("Alt")
  if (c.shift) parts.push("Shift")
  if (c.cmd) parts.push("CmdOrCtrl")
  parts.push(ACCEL_KEY[c.key] ?? c.key.toUpperCase())
  return parts.join("+")
}

const MAC_KEY: Record<string, string> = { comma: ",", backspace: "⌫", enter: "↩", period: "." }
const PC_KEY: Record<string, string> = {
  comma: ",",
  backspace: "Backspace",
  enter: "Enter",
  period: ".",
}

/** The hint shown next to a command: "⇧⌘R" on a Mac, "Ctrl+Shift+R" elsewhere. */
export function displayChord(chord: string, mac = isMacPlatform()): string {
  const c = parseChord(chord)
  if (mac) {
    return `${c.ctrl ? "⌃" : ""}${c.alt ? "⌥" : ""}${c.shift ? "⇧" : ""}${c.cmd ? "⌘" : ""}${MAC_KEY[c.key] ?? c.key.toUpperCase()}`
  }
  const parts: string[] = []
  if (c.cmd || c.ctrl) parts.push("Ctrl")
  if (c.alt) parts.push("Alt")
  if (c.shift) parts.push("Shift")
  parts.push(PC_KEY[c.key] ?? c.key.toUpperCase())
  return parts.join("+")
}
