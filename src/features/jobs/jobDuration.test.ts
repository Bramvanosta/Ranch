/**
 * `jobDurationMs` rules.
 */
import { describe, expect, it } from "vitest"

import type { Job } from "@/lib/tauri/bindings"

import { jobDurationMs } from "./jobDuration"

function base(overrides: Partial<Job> = {}): Job {
  return {
    jobRef: { queueId: "bull:q", queueName: "q", id: "1" },
    name: "n",
    state: "waiting",
    data: "{}",
    opts: "{}",
    attemptsMade: 0,
    timestamp: 1_700_000_000_000,
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

const NOW = 1_700_000_100_000

describe("jobDurationMs", () => {
  it("returns null when processedOn is null", () => {
    expect(jobDurationMs(base(), NOW)).toBeNull()
  })

  it("returns null for waiting jobs even if processedOn was somehow set", () => {
    expect(jobDurationMs(base({ processedOn: 0 }), NOW)).toBeNull()
  })

  it("returns finishedOn − processedOn for completed jobs", () => {
    expect(
      jobDurationMs(
        base({ state: "completed", processedOn: NOW - 3_000, finishedOn: NOW - 1_000 }),
        NOW,
      ),
    ).toBe(2_000)
  })

  it("returns finishedOn − processedOn for failed jobs", () => {
    expect(
      jobDurationMs(
        base({ state: "failed", processedOn: NOW - 5_000, finishedOn: NOW - 500 }),
        NOW,
      ),
    ).toBe(4_500)
  })

  it("returns null for completed jobs missing finishedOn", () => {
    expect(jobDurationMs(base({ state: "completed", processedOn: NOW - 1_000 }), NOW)).toBeNull()
  })

  it("ticks with now for active jobs", () => {
    expect(jobDurationMs(base({ state: "active", processedOn: NOW - 1_200 }), NOW)).toBe(1_200)
  })

  it("clamps negative durations to 0 (clock skew safety)", () => {
    expect(
      jobDurationMs(base({ state: "completed", processedOn: NOW, finishedOn: NOW - 100 }), NOW),
    ).toBe(0)
  })
})
