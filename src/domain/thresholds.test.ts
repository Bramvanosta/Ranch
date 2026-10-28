import { describe, expect, it } from "vitest"

import {
  isThresholdCrossing,
  type NotificationThreshold,
  thresholdKey,
  thresholdMatches,
} from "./thresholds"

const T: NotificationThreshold = {
  id: "t1",
  metric: "failed",
  op: "gt",
  value: 10,
}

describe("isThresholdCrossing", () => {
  it("returns false when current is below threshold", () => {
    expect(isThresholdCrossing(T, 5, 5)).toBe(false)
    expect(isThresholdCrossing(T, 5, 10)).toBe(false)
  })
  it("returns true when crossing from below to above", () => {
    expect(isThresholdCrossing(T, 5, 11)).toBe(true)
    expect(isThresholdCrossing(T, 10, 11)).toBe(true)
  })
  it("returns false when staying above threshold", () => {
    expect(isThresholdCrossing(T, 12, 15)).toBe(false)
    expect(isThresholdCrossing(T, 11, 11)).toBe(false)
  })
  it("fires on first observation if already above (initial-state notify)", () => {
    expect(isThresholdCrossing(T, undefined, 20)).toBe(true)
    expect(isThresholdCrossing(T, undefined, 5)).toBe(false)
  })
})

describe("thresholdKey", () => {
  it("stably composes connection + queue + metric", () => {
    expect(thresholdKey("c", "q", "failed")).toBe("c::q::failed")
  })
})

describe("thresholdMatches", () => {
  it("matches global (no scope)", () => {
    expect(thresholdMatches(T, "c1", "emails")).toBe(true)
  })
  it("respects connectionId scope", () => {
    const t = { ...T, connectionId: "c2" }
    expect(thresholdMatches(t, "c1", "emails")).toBe(false)
    expect(thresholdMatches(t, "c2", "emails")).toBe(true)
  })
  it("respects queueName scope", () => {
    const t = { ...T, queueName: "orders" }
    expect(thresholdMatches(t, "c1", "emails")).toBe(false)
    expect(thresholdMatches(t, "c1", "orders")).toBe(true)
  })
})
