import { describe, expect, it } from "vitest"

import { formatEveryMs } from "./formatEvery"

describe("formatEveryMs", () => {
  it("formats sub-second durations as ms", () => {
    expect(formatEveryMs(0)).toBe("0ms")
    expect(formatEveryMs(999)).toBe("999ms")
  })

  it("formats sub-minute durations as s", () => {
    expect(formatEveryMs(1_000)).toBe("1s")
    expect(formatEveryMs(59_999)).toBe("59s")
  })

  it("formats sub-hour durations as m", () => {
    expect(formatEveryMs(60_000)).toBe("1m")
    expect(formatEveryMs(15 * 60_000)).toBe("15m")
    expect(formatEveryMs(60 * 60_000 - 1)).toBe("59m")
  })

  it("formats sub-day durations as h", () => {
    expect(formatEveryMs(60 * 60_000)).toBe("1h")
    expect(formatEveryMs(23 * 60 * 60_000)).toBe("23h")
  })

  it("formats >=1 day as d", () => {
    expect(formatEveryMs(24 * 60 * 60_000)).toBe("1d")
    expect(formatEveryMs(7 * 24 * 60 * 60_000)).toBe("7d")
  })

  it("returns em-dash for negative or non-finite input", () => {
    expect(formatEveryMs(-1)).toBe("—")
    expect(formatEveryMs(Number.NaN)).toBe("—")
    expect(formatEveryMs(Number.POSITIVE_INFINITY)).toBe("—")
  })
})
