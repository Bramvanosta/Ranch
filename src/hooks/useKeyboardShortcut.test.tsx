import { renderHook } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { useKeyboardShortcut } from "./useKeyboardShortcut"

const originalPlatform = navigator.platform

function setPlatform(platform: string) {
  Object.defineProperty(navigator, "platform", { value: platform, configurable: true })
}

afterEach(() => {
  setPlatform(originalPlatform)
})

function fireChord(opts: {
  key: string
  metaKey?: boolean
  ctrlKey?: boolean
  shiftKey?: boolean
  altKey?: boolean
}) {
  const event = new KeyboardEvent("keydown", {
    key: opts.key,
    metaKey: opts.metaKey ?? false,
    ctrlKey: opts.ctrlKey ?? false,
    shiftKey: opts.shiftKey ?? false,
    altKey: opts.altKey ?? false,
  })
  window.dispatchEvent(event)
}

describe("useKeyboardShortcut", () => {
  describe("on Mac", () => {
    beforeEach(() => setPlatform("MacIntel"))

    it("fires the handler on cmd+shift+s", () => {
      const handler = vi.fn()
      renderHook(() => useKeyboardShortcut("cmd+shift+s", handler))
      fireChord({ key: "s", metaKey: true, shiftKey: true })
      expect(handler).toHaveBeenCalledTimes(1)
    })

    it("does NOT fire on ctrl+shift+s (Mac uses cmd)", () => {
      const handler = vi.fn()
      renderHook(() => useKeyboardShortcut("cmd+shift+s", handler))
      fireChord({ key: "s", ctrlKey: true, shiftKey: true })
      expect(handler).not.toHaveBeenCalled()
    })

    it("does NOT fire when shift is missing", () => {
      const handler = vi.fn()
      renderHook(() => useKeyboardShortcut("cmd+shift+s", handler))
      fireChord({ key: "s", metaKey: true })
      expect(handler).not.toHaveBeenCalled()
    })
  })

  describe("on Linux/Windows", () => {
    beforeEach(() => setPlatform("Linux x86_64"))

    it("`cmd` chord resolves to ctrl+shift+s on non-Mac", () => {
      const handler = vi.fn()
      renderHook(() => useKeyboardShortcut("cmd+shift+s", handler))
      fireChord({ key: "s", ctrlKey: true, shiftKey: true })
      expect(handler).toHaveBeenCalledTimes(1)
    })

    it("does NOT fire on cmd (meta) on non-Mac", () => {
      const handler = vi.fn()
      renderHook(() => useKeyboardShortcut("cmd+shift+s", handler))
      fireChord({ key: "s", metaKey: true, shiftKey: true })
      expect(handler).not.toHaveBeenCalled()
    })
  })

  it("removes the listener on unmount", () => {
    setPlatform("MacIntel")
    const handler = vi.fn()
    const { unmount } = renderHook(() => useKeyboardShortcut("cmd+shift+s", handler))
    unmount()
    fireChord({ key: "s", metaKey: true, shiftKey: true })
    expect(handler).not.toHaveBeenCalled()
  })
})
