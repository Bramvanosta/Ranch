/**
 * Formatter unit tests. Pure functions, no DOM.
 */
import { describe, expect, it } from "vitest"

import { formatCount, formatDurationMs, formatPercent, formatThroughput } from "./format"

describe("formatDurationMs", () => {
  it("renders a dash when value is zero or negative", () => {
    expect(formatDurationMs(0)).toBe("—")
    expect(formatDurationMs(-1)).toBe("—")
    expect(formatDurationMs(Number.NaN)).toBe("—")
  })

  it("uses ms below one second", () => {
    expect(formatDurationMs(42)).toBe("42 ms")
    expect(formatDurationMs(999)).toBe("999 ms")
  })

  it("uses seconds with 2 decimals below 10s", () => {
    expect(formatDurationMs(2_500)).toBe("2.50 s")
  })

  it("uses seconds with 1 decimal in 10s..1min", () => {
    expect(formatDurationMs(15_000)).toBe("15.0 s")
  })

  it("uses minutes above one minute", () => {
    expect(formatDurationMs(90_000)).toBe("1.5 min")
  })

  it("uses hours above one hour", () => {
    expect(formatDurationMs(3_600_000 * 2)).toBe("2.0 h")
  })
})

describe("formatPercent", () => {
  it("renders 0%", () => {
    expect(formatPercent(0)).toBe("0%")
  })

  it("uses 2 decimals below 1%", () => {
    expect(formatPercent(0.0025)).toBe("0.25%")
  })

  it("uses 1 decimal between 1% and 10%", () => {
    expect(formatPercent(0.054)).toBe("5.4%")
  })

  it("rounds at or above 10%", () => {
    expect(formatPercent(0.125)).toBe("13%")
  })

  it("dashes invalid input", () => {
    expect(formatPercent(Number.NaN)).toBe("—")
  })
})

describe("formatThroughput", () => {
  it("renders 0/h for zero or invalid", () => {
    expect(formatThroughput(0)).toBe("0/h")
    expect(formatThroughput(Number.NaN)).toBe("0/h")
  })

  it("uses 1 decimal below 10", () => {
    expect(formatThroughput(4.2)).toBe("4.2/h")
  })

  it("rounds in tens..thousands", () => {
    expect(formatThroughput(125)).toBe("125/h")
  })

  it("uses 'k/h' above 1000", () => {
    expect(formatThroughput(2_500)).toBe("2.5k/h")
  })
})

describe("formatCount", () => {
  it("renders 0", () => {
    expect(formatCount(0)).toBe("0")
  })

  it("separates thousands", () => {
    expect(formatCount(1_234)).toBe("1,234")
    expect(formatCount(1_234_567)).toBe("1,234,567")
  })
})
