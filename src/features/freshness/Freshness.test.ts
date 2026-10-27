/**
 * How fresh a screen's data is.
 */
import { describe, expect, it } from "vitest"

import { freshnessOf } from "./Freshness"

const NOW = 1_000_000
const ok = (ageMs: number) => ({ dataUpdatedAt: NOW - ageMs, errorUpdatedAt: 0, error: null })

describe("freshnessOf", () => {
  it("is loading before anything was read", () => {
    expect(freshnessOf([], 1_000, NOW).kind).toBe("loading")
    expect(
      freshnessOf([{ dataUpdatedAt: 0, errorUpdatedAt: 0, error: null }], 1_000, NOW).kind,
    ).toBe("loading")
  })

  it("is live within twice the interval, from the oldest query", () => {
    expect(freshnessOf([ok(500), ok(1_500)], 1_000, NOW)).toEqual({ kind: "live", ageMs: 1_500 })
  })

  it("is stale past twice the interval", () => {
    expect(freshnessOf([ok(9_000)], 2_000, NOW).kind).toBe("stale")
  })

  it("never goes stale with auto-refresh off", () => {
    expect(freshnessOf([ok(600_000)], false, NOW).kind).toBe("live")
  })

  it("says the last refresh failed, with its reason, over its age", () => {
    const failing = {
      dataUpdatedAt: NOW - 40_000,
      errorUpdatedAt: NOW - 1_000,
      error: new Error("timed out"),
    }
    expect(freshnessOf([ok(500), failing], 1_000, NOW)).toEqual({
      kind: "failing",
      ageMs: 40_000,
      reason: "timed out",
    })
  })

  it("ignores a query that never read anything (an offline connection)", () => {
    const offline = { dataUpdatedAt: 0, errorUpdatedAt: NOW, error: new Error("refused") }
    expect(freshnessOf([ok(500), offline], 1_000, NOW)).toEqual({ kind: "live", ageMs: 500 })
  })

  it("is down when it failed before reading anything", () => {
    const down = { dataUpdatedAt: 0, errorUpdatedAt: NOW, error: new Error("refused") }
    expect(freshnessOf([down], 1_000, NOW)).toEqual({ kind: "down", reason: "refused" })
  })
})
