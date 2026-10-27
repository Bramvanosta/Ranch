import { describe, expect, it } from "vitest"

import { parseJobProgress } from "./parseJobProgress"

describe("parseJobProgress", () => {
  it("returns null when nothing was reported", () => {
    expect(parseJobProgress(null)).toBeNull()
    expect(parseJobProgress(undefined)).toBeNull()
    expect(parseJobProgress("  ")).toBeNull()
    expect(parseJobProgress("null")).toBeNull()
  })

  it("reads a number as a clamped percentage", () => {
    expect(parseJobProgress("42")).toEqual({ percent: 42, label: "42%", note: null })
    expect(parseJobProgress("66.6")).toEqual({ percent: 66.6, label: "67%", note: null })
    expect(parseJobProgress("250")).toEqual({ percent: 100, label: "100%", note: null })
    expect(parseJobProgress("-3")).toEqual({ percent: 0, label: "0%", note: null })
  })

  it("reads a percentage and a note out of an object", () => {
    expect(parseJobProgress('{"step":"upload","pct":40}')).toEqual({
      percent: 40,
      label: "40%",
      note: "upload",
    })
    expect(parseJobProgress('{"percentage":12.4,"message":"page 3 of 24"}')).toEqual({
      percent: 12.4,
      label: "12%",
      note: "page 3 of 24",
    })
  })

  it("shows other objects and strings as text without interpreting them", () => {
    expect(parseJobProgress('{"done":3,"total":9}')).toEqual({
      percent: null,
      label: '{"done":3,"total":9}',
      note: null,
    })
    expect(parseJobProgress('"resizing"')).toEqual({
      percent: null,
      label: "resizing",
      note: null,
    })
    expect(parseJobProgress("not json")).toEqual({ percent: null, label: "not json", note: null })
  })

  it("keeps long values whole: the row wraps", () => {
    const long = JSON.stringify({ detail: "x".repeat(200) })
    expect(parseJobProgress(long)?.label).toBe(long)
  })
})
