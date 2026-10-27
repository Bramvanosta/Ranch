import { describe, expect, it } from "vitest"

import {
  DEFAULT_REFRESH_SETTINGS,
  effectiveInterval,
  intervalLabel,
  parseRefreshSettings,
  REFRESH_INTERVALS_MS,
} from "./refreshSettings"

describe("refresh settings", () => {
  it("defaults to what Ranch always did: on, every second", () => {
    expect(parseRefreshSettings(null, null)).toEqual(DEFAULT_REFRESH_SETTINGS)
    expect(DEFAULT_REFRESH_SETTINGS).toEqual({ auto: true, intervalMs: 1_000 })
  })

  it("reads the stored values and ignores anything unexpected", () => {
    expect(parseRefreshSettings("0", "10000")).toEqual({ auto: false, intervalMs: 10_000 })
    expect(parseRefreshSettings("1", "5000")).toEqual({ auto: true, intervalMs: 5_000 })
    expect(parseRefreshSettings("1", "1234").intervalMs).toBe(1_000)
    expect(parseRefreshSettings("1", "fast").intervalMs).toBe(1_000)
  })

  it("never polls when off, and never faster than a view's floor", () => {
    expect(effectiveInterval({ auto: false, intervalMs: 1_000 }, 1_000)).toBe(false)
    expect(effectiveInterval({ auto: true, intervalMs: 1_000 }, 1_000)).toBe(1_000)
    expect(effectiveInterval({ auto: true, intervalMs: 1_000 }, 5_000)).toBe(5_000)
    expect(effectiveInterval({ auto: true, intervalMs: 30_000 }, 5_000)).toBe(30_000)
  })

  it("offers 1 / 2 / 5 / 10 / 30 s", () => {
    expect(REFRESH_INTERVALS_MS.map(intervalLabel)).toEqual(["1 s", "2 s", "5 s", "10 s", "30 s"])
  })
})
