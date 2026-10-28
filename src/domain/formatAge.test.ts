import { describe, expect, it } from "vitest"

import { formatAge } from "./formatAge"

describe("formatAge", () => {
  it('returns "now" for negative durations (clock skew)', () => {
    expect(formatAge(-100)).toBe("now")
    expect(formatAge(-1_000_000)).toBe("now")
  })

  it("formats sub-minute durations as seconds", () => {
    expect(formatAge(0)).toBe("0s")
    expect(formatAge(1_000)).toBe("1s")
    expect(formatAge(59_000)).toBe("59s")
  })

  it("formats sub-hour durations as minutes", () => {
    expect(formatAge(60_000)).toBe("1m")
    expect(formatAge(60 * 60_000 - 1)).toBe("59m")
  })

  it("formats sub-day durations as hours", () => {
    expect(formatAge(60 * 60_000)).toBe("1h")
    expect(formatAge(23 * 60 * 60_000)).toBe("23h")
  })

  it("formats >=1 day as days", () => {
    expect(formatAge(24 * 60 * 60_000)).toBe("1d")
    expect(formatAge(7 * 24 * 60 * 60_000)).toBe("7d")
  })

  it('handles non-finite input as "now"', () => {
    expect(formatAge(Number.NaN)).toBe("now")
    expect(formatAge(Number.POSITIVE_INFINITY)).toBe("now")
  })
})
