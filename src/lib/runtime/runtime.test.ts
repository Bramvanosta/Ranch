import { afterEach, describe, expect, it, vi } from "vitest"

import {
  hasAgents,
  hasLiveEvents,
  hasNativeMenus,
  hasNativeWindows,
  hasOverlayTitleBar,
  hasUpdater,
  isRanchServer,
  type Runtime,
  setRuntime,
} from "./runtime"

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

describe("runtime capabilities", () => {
  afterEach(() => setRuntime("preview"))

  const capabilities = (runtime: Runtime) => {
    setRuntime(runtime)
    return {
      agents: hasAgents(),
      updater: hasUpdater(),
      nativeWindows: hasNativeWindows(),
      nativeMenus: hasNativeMenus(),
      liveEvents: hasLiveEvents(),
      ranchServer: isRanchServer(),
    }
  }

  it("gives the desktop app everything native", () => {
    expect(capabilities("desktop")).toEqual({
      agents: true,
      updater: true,
      nativeWindows: true,
      nativeMenus: true,
      liveEvents: true,
      ranchServer: false,
    })
  })

  it("keeps the server image to what works over HTTP", () => {
    expect(capabilities("server")).toEqual({
      agents: false,
      updater: false,
      nativeWindows: false,
      nativeMenus: false,
      liveEvents: false,
      ranchServer: true,
    })
  })

  it("shows agents and updates in the preview, without native windows or live events", () => {
    expect(capabilities("preview")).toEqual({
      agents: true,
      updater: true,
      nativeWindows: false,
      nativeMenus: false,
      liveEvents: false,
      ranchServer: false,
    })
  })
})
