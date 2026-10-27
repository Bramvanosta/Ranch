import { describe, expect, it } from "vitest"
import { fuzzyScore, rankBy } from "./fuzzyMatch"

describe("fuzzyScore", () => {
  it("returns 1 for empty needle", () => {
    expect(fuzzyScore("", "anything")).toBe(1)
  })
  it("returns 1000 for an exact (case-insensitive) match", () => {
    expect(fuzzyScore("Foo", "foo")).toBe(1000)
  })
  it("returns 500 for a prefix match", () => {
    expect(fuzzyScore("emai", "emails-prod")).toBe(500)
  })
  it("returns 0 for no match", () => {
    expect(fuzzyScore("xyz", "no-match-here")).toBe(0)
  })
  it("ranks earlier substring higher than later", () => {
    const earlier = fuzzyScore("ord", "orders-q")
    const later = fuzzyScore("ord", "z-orders-q")
    expect(earlier).toBeGreaterThan(later)
  })
  it("scores subsequence match positively", () => {
    expect(fuzzyScore("emp", "email-prod")).toBeGreaterThan(0)
  })
})

describe("rankBy", () => {
  it("returns matches in score-desc order", () => {
    const items = ["audit", "orders", "emails", "audit-log"]
    expect(rankBy(items, "aud", (s) => s)).toEqual(["audit", "audit-log"])
  })
  it("excludes zero-score items", () => {
    expect(rankBy(["x", "y", "abc"], "z", (s) => s)).toEqual([])
  })
})
