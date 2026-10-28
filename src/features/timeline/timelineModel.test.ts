import { describe, expect, it } from "vitest"

import type { Throughput } from "@/lib/tauri/bindings"

import {
  formatWait,
  fraction,
  latestMoment,
  placeMarkers,
  queueRows,
  spanOf,
  ticks,
  timeAt,
  worstMoment,
} from "./timelineModel"

const queue = (name: string, completed: number[], failed: number[], metricsOff = false) => ({
  queueId: `bull:${name}`,
  queueName: name,
  metricsOff,
  completed,
  failed,
})

const t: Throughput = {
  range: "1h",
  bucketMs: 600_000,
  bucketStartsMs: [0, 600_000, 1_200_000],
  queues: [
    queue("quiet", [1, 0, 0], [0, 0, 0]),
    queue("busy", [10, 20, 5], [0, 3, 0]),
    queue("off", [], [], true),
    queue("mid", [5, 5, 5], [1, 1, 1]),
  ],
  completed: [16, 25, 10],
  failed: [1, 4, 1],
  metricsOffQueues: 1,
} as unknown as Throughput

describe("timeline model", () => {
  it("lists the queues with metrics, most failures first, on one scale", () => {
    const { rows, scale } = queueRows(t)
    expect(rows.map((r) => r.name)).toEqual(["busy", "mid", "quiet"])
    expect(rows[0]).toMatchObject({ done: 35, failures: 3 })
    expect(scale).toBe(23)
  })

  it("picks the worst moment, widened to neighbours failing at least half as much", () => {
    expect(worstMoment(t)).toEqual({ from: 600_000, to: 1_200_000 })
    const spread = { ...t, failed: [3, 4, 0] } as Throughput
    expect(worstMoment(spread)).toEqual({ from: 0, to: 1_200_000 })
    expect(worstMoment({ ...t, failed: [0, 0, 0] } as Throughput)).toBeNull()
  })

  it("falls back on the latest twenty-fourth of the range", () => {
    expect(latestMoment({ startMs: 0, endMs: 2_400_000 })).toEqual({
      from: 2_300_000,
      to: 2_400_000,
    })
  })

  it("maps time to the axis and back", () => {
    const span = spanOf(t)
    expect(span).toEqual({ startMs: 0, endMs: 1_800_000 })
    expect(fraction(span, 900_000)).toBe(0.5)
    expect(fraction(span, -5)).toBe(0)
    expect(timeAt(span, 0.25)).toBe(450_000)
    expect(ticks(span).length).toBeGreaterThan(2)
  })

  it("puts markers on rows so labels never overlap", () => {
    const span = { startMs: 0, endMs: 1000 }
    const m = (key: string, atMs: number) => ({
      key,
      atMs,
      label: key,
      kind: "audit" as const,
      title: key,
    })
    const placed = placeMarkers([m("a", 100), m("b", 120), m("c", 500)], span, () => 0.1)
    expect(placed.map((p) => [p.key, p.row])).toEqual([
      ["a", 0],
      ["b", 1],
      ["c", 0],
    ])
  })

  it("says waits in the right unit", () => {
    expect(formatWait(null)).toBe("-")
    expect(formatWait(420)).toBe("420 ms")
    expect(formatWait(4200)).toBe("4.2 s")
    expect(formatWait(600_000)).toBe("10 min")
  })
})
