import { render, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { displayChord, matchesChord, toAccelerator } from "./chords"
import { commandState, runCommand, useCommandHandler, useCommandHandlers } from "./handlers"
import { COMMANDS } from "./registry"

const key = (code: string, mods: Partial<KeyboardEventInit> = {}) =>
  new KeyboardEvent("keydown", { code, bubbles: true, ...mods })

describe("chords", () => {
  it("reads one notation three ways, on a Mac", () => {
    expect(displayChord("cmd+shift+r", true)).toBe("⇧⌘R")
    expect(displayChord("alt+cmd+p", true)).toBe("⌥⌘P")
    expect(displayChord("ctrl+cmd+s", true)).toBe("⌃⌘S")
    expect(displayChord("cmd+backspace", true)).toBe("⌘⌫")
    expect(toAccelerator("cmd+shift+r", true)).toBe("Shift+CmdOrCtrl+R")
    expect(toAccelerator("cmd+comma", true)).toBe("CmdOrCtrl+,")
    expect(matchesChord(key("KeyR", { metaKey: true, shiftKey: true }), "cmd+shift+r", true)).toBe(
      true,
    )
    // ⌘R is Refresh, not Retry.
    expect(matchesChord(key("KeyR", { metaKey: true }), "cmd+shift+r", true)).toBe(false)
    // ⌥P types "π": the key code still says P.
    expect(matchesChord(key("KeyP", { metaKey: true, altKey: true }), "alt+cmd+p", true)).toBe(true)
  })

  it("maps ⌘ to Ctrl elsewhere", () => {
    expect(displayChord("cmd+shift+r", false)).toBe("Ctrl+Shift+R")
    expect(toAccelerator("ctrl+cmd+s", false)).toBe("CmdOrCtrl+S")
    expect(matchesChord(key("KeyR", { ctrlKey: true, shiftKey: true }), "cmd+shift+r", false)).toBe(
      true,
    )
  })

  it("never binds a chord the platform owns", () => {
    const chords = COMMANDS.flatMap((c) => (c.shortcut ? [c.shortcut] : []))
    for (const reserved of ["cmd+p", "cmd+shift+s", "cmd+q", "cmd+h", "cmd+w", "cmd+m", "cmd+c"]) {
      expect(chords).not.toContain(reserved)
    }
    expect(new Set(chords).size).toBe(chords.length)
  })
})

describe("command handlers", () => {
  afterEach(() => useCommandHandlers.setState({ handlers: {} }))

  it("a command applies only while something handles it", () => {
    const run = vi.fn()
    const { unmount } = renderHook(() => useCommandHandler("job.retry", run))
    expect(commandState(useCommandHandlers.getState().handlers, "job.retry")).toEqual({
      enabled: true,
      label: "Retry",
    })
    expect(runCommand("job.retry")).toBe(true)
    expect(run).toHaveBeenCalledTimes(1)
    unmount()
    expect(commandState(useCommandHandlers.getState().handlers, "job.retry").enabled).toBe(false)
    expect(runCommand("job.retry")).toBe(false)
  })

  it("the selection outranks the focused row, and says so", () => {
    const row = vi.fn()
    const selection = vi.fn()
    renderHook(() => useCommandHandler("job.retry", row, { priority: 1 }))
    renderHook(() =>
      useCommandHandler("job.retry", selection, { priority: 2, label: "Retry 3 Jobs" }),
    )
    expect(commandState(useCommandHandlers.getState().handlers, "job.retry").label).toBe(
      "Retry 3 Jobs",
    )
    runCommand("job.retry")
    expect(selection).toHaveBeenCalled()
    expect(row).not.toHaveBeenCalled()
  })

  it("a disabled handler greys the command out", () => {
    renderHook(() => useCommandHandler("queue.pause", vi.fn(), { enabled: false }))
    expect(commandState(useCommandHandlers.getState().handlers, "queue.pause").enabled).toBe(false)
    expect(runCommand("queue.pause")).toBe(false)
  })

  it("the window's chord runs the command, but ⌘⌫ in a text field stays a text edit", () => {
    const remove = vi.fn()
    const refresh = vi.fn()
    // The window listener is installed once by the test setup, as the app's root does.
    function Harness() {
      useCommandHandler("job.remove", remove)
      useCommandHandler("view.refresh", refresh)
      return <input aria-label="field" />
    }
    const { getByLabelText } = render(<Harness />)
    const meta = /Mac/.test(navigator.platform) ? { metaKey: true } : { ctrlKey: true }
    window.dispatchEvent(key("KeyR", meta))
    expect(refresh).toHaveBeenCalledTimes(1)
    getByLabelText("field").dispatchEvent(key("Backspace", meta))
    expect(remove).not.toHaveBeenCalled()
  })

  it("one press seen by the native menu and the key listener runs once", () => {
    const settings = vi.fn()
    renderHook(() => useCommandHandler("app.settings", settings))
    runCommand("app.settings", "menu")
    runCommand("app.settings", "key")
    expect(settings).toHaveBeenCalledTimes(1)
  })
})
