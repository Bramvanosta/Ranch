import { afterEach, describe, expect, it, vi } from "vitest"

import { hasOverlayTitleBar, setRuntime } from "./runtime"

describe("hasOverlayTitleBar", () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    setRuntime("preview")
  })

  const as = (userAgent: string) => vi.stubGlobal("navigator", { ...navigator, userAgent })

  it("is true only for the desktop app on macOS", () => {
    as("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15")
    setRuntime("desktop")
    expect(hasOverlayTitleBar()).toBe(true)
  })

  it("keeps native decorations on Linux", () => {
    as("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15")
    setRuntime("desktop")
    expect(hasOverlayTitleBar()).toBe(false)
  })

  it("has no window chrome in a browser, even on a Mac", () => {
    as("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15")
    setRuntime("server")
    expect(hasOverlayTitleBar()).toBe(false)
    setRuntime("preview")
    expect(hasOverlayTitleBar()).toBe(false)
  })
})
