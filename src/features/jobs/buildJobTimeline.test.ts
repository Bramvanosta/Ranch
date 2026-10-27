import { describe, expect, it } from "vitest"

import type { Job } from "@/lib/tauri/bindings"

import { buildJobTimeline, formatStepTime } from "./buildJobTimeline"

const NOW = new Date(2026, 8, 19, 10, 30, 0).getTime()

function job(overrides: Partial<Job> = {}): Job {
  return {
    jobRef: { queue: { prefix: "bull", name: "emails" }, id: "42" },
    name: "send-email",
    state: "waiting",
    data: "{}",
    opts: "{}",
    attemptsMade: 0,
    timestamp: NOW - 10 * 60_000,
    processedOn: null,
    finishedOn: null,
    delay: null,
    failedReason: null,
    stacktrace: [],
    returnValue: null,
    parentKey: null,
    progress: null,
    ...overrides,
  }
}

const summary = (j: Job) => buildJobTimeline(j, NOW).map((s) => [s.key, s.label, s.tone, s.note])

describe("buildJobTimeline", () => {
  it("a waiting job: added, then what it is waiting for", () => {
    expect(summary(job())).toEqual([
      ["added", "Added", "done", "10m ago"],
      ["pending", "Waiting for a worker", "pending", null],
    ])
  })

  it("a completed job: how long it waited, how long it ran", () => {
    const t = NOW - 10 * 60_000
    expect(
      summary(
        job({
          state: "completed",
          attemptsMade: 1,
          processedOn: t + 2_100,
          finishedOn: t + 2_100 + 6_880,
        }),
      ),
    ).toEqual([
      ["added", "Added", "done", "10m ago"],
      ["started", "Started", "done", "waited 2.10 s"],
      ["finished", "Completed", "completed", "ran 6.88 s"],
    ])
  })

  it("a job that failed for good names the attempt `processedOn` belongs to", () => {
    const t = NOW - 10 * 60_000
    const steps = summary(
      job({ state: "failed", attemptsMade: 3, processedOn: t + 90_000, finishedOn: t + 91_500 }),
    )
    expect(steps[1]).toEqual([
      "started",
      "Started · attempt 3",
      "done",
      "1.5 min after it was added",
    ])
    expect(steps[2]).toEqual(["finished", "Failed", "failed", "ran 1.50 s"])
  })

  it("a running job ticks: the attempt in progress is attemptsMade + 1", () => {
    const steps = summary(job({ state: "active", attemptsMade: 1, processedOn: NOW - 12_000 }))
    expect(steps[1]?.[1]).toBe("Started · attempt 2")
    expect(steps[2]).toEqual(["pending", "Running", "active", "for 12.0 s"])
  })

  it("a delayed job that never ran says when it is due", () => {
    const steps = buildJobTimeline(job({ state: "delayed", delay: 15 * 60_000 }), NOW)
    expect(steps.map((s) => s.key)).toEqual(["added", "due"])
    expect(steps[1]?.at).toBe(NOW + 5 * 60_000)
    expect(steps[1]?.note).toBe("in 5.0 min")
  })

  it("a delayed job past its time says so", () => {
    const steps = buildJobTimeline(job({ state: "delayed", delay: 9 * 60_000 }), NOW)
    expect(steps[1]?.note).toBe("1.0 min ago — no worker took it yet")
  })

  it("a retry waiting for its backoff has no due time: the hash does not keep it", () => {
    const steps = buildJobTimeline(
      job({ state: "delayed", attemptsMade: 1, processedOn: NOW - 60_000, delay: 4_000 }),
      NOW,
    )
    expect(steps.map((s) => [s.key, s.label, s.at, s.note])).toEqual([
      ["added", "Added", NOW - 10 * 60_000, "10m ago"],
      ["pending", "Retry scheduled", null, "after a 4.00 s backoff"],
    ])
  })

  it("a parent waiting for its children", () => {
    expect(summary(job({ state: "waitingChildren" }))[1]).toEqual([
      "pending",
      "Waiting for its children",
      "pending",
      null,
    ])
  })

  it("keeps the end of a finished job whose start was not recorded", () => {
    const steps = summary(job({ state: "completed", finishedOn: NOW - 1_000 }))
    expect(steps.map((s) => s[0])).toEqual(["added", "finished"])
    expect(steps[1]?.[3]).toBeNull()
  })
})

describe("formatStepTime", () => {
  it("writes the date once: later steps of the same day show the time", () => {
    const first = new Date(2026, 8, 19, 9, 59, 12).getTime()
    expect(formatStepTime(first, first)).toBe("2026-09-19 09:59:12")
    expect(formatStepTime(first + 2_000, first)).toBe("09:59:14")
    expect(formatStepTime(first + 24 * 3_600_000, first)).toBe("2026-09-20 09:59:12")
  })
})
