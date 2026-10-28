import { describe, expect, it } from "vitest"
import { BULLMQ_CAPABILITIES } from "@/backend/fake/backends"

import type { Job } from "@/lib/tauri/bindings"

import { backoffMs, buildJobTrace } from "./buildJobTrace"

const BULLMQ = BULLMQ_CAPABILITIES.jobStates
const t0 = 1_000_000

function job(overrides: Partial<Job>): Job {
  return {
    jobRef: { queueId: "bull:emails", queueName: "emails", id: "1" },
    name: "send",
    state: "waiting",
    data: "{}",
    opts: "{}",
    attemptsMade: 0,
    timestamp: t0,
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

describe("buildJobTrace", () => {
  it("one attempt: the stored times, nothing estimated", () => {
    const trace = buildJobTrace(
      job({ state: "failed", attemptsMade: 1, processedOn: t0 + 2_000, finishedOn: t0 + 3_000 }),
      {},
      t0 + 60_000,
      BULLMQ,
    )
    expect(trace.estimated).toBe(false)
    expect(trace.rows.map((r) => r.key)).toEqual(["waiting", "attempt-1"])
    expect(trace.rows[0]).toMatchObject({ start: t0, end: t0 + 2_000, kind: "wait" })
    expect(trace.rows[1]).toMatchObject({ kind: "work", tone: "failed" })
    expect(trace.start).toBe(t0)
    expect(trace.end).toBe(t0 + 3_000)
    expect(trace.summary).toBe("Added → failed in 3.00 s · 1.00 s of actual work")
  })

  it("three attempts: earlier ones placed back from the last with the exponential backoff", () => {
    const trace = buildJobTrace(
      job({
        state: "failed",
        attemptsMade: 3,
        processedOn: t0 + 20_000,
        finishedOn: t0 + 21_000,
      }),
      { attempts: 3, backoff: { type: "exponential", delay: 1_000 } },
      t0 + 60_000,
      BULLMQ,
    )
    expect(trace.rows.map((r) => r.key)).toEqual([
      "waiting",
      "attempt-1",
      "backoff-1",
      "attempt-2",
      "backoff-2",
      "attempt-3",
    ])
    const byKey = Object.fromEntries(trace.rows.map((r) => [r.key, r]))
    // Last attempt: exact. Backoff 2 = 2 s, backoff 1 = 1 s, attempts of 1 s each.
    expect(byKey["attempt-3"]).toMatchObject({ start: t0 + 20_000, end: t0 + 21_000 })
    expect(byKey["attempt-3"].estimated).toBeUndefined()
    expect(byKey["backoff-2"]).toMatchObject({ start: t0 + 18_000, end: t0 + 20_000 })
    expect(byKey["attempt-2"]).toMatchObject({ start: t0 + 17_000, estimated: true })
    expect(byKey["backoff-1"]).toMatchObject({ start: t0 + 16_000, end: t0 + 17_000 })
    expect(byKey["attempt-1"]).toMatchObject({ start: t0 + 15_000, tone: "failed" })
    expect(byKey.waiting).toMatchObject({ start: t0, end: t0 + 15_000, estimated: true })
    expect(trace.estimated).toBe(true)
    expect(trace.summary).toBe("Added → failed in 21.0 s · ≈ 3.00 s of actual work")
  })

  it("a running job ends now and says so", () => {
    const trace = buildJobTrace(
      job({ state: "active", attemptsMade: 0, processedOn: t0 + 1_000 }),
      {},
      t0 + 4_000,
      BULLMQ,
    )
    const last = trace.rows[trace.rows.length - 1]
    expect(last).toMatchObject({ key: "attempt-1", tone: "running", end: t0 + 4_000 })
    expect(last?.detail).toBe("running · 3.00 s")
    expect(trace.summary).toBe("Added 4.00 s ago · 3.00 s of actual work so far")
  })

  it("a job that never ran is one pending row", () => {
    const trace = buildJobTrace(job({ state: "waiting" }), {}, t0 + 5_000, BULLMQ)
    expect(trace.rows).toHaveLength(1)
    expect(trace.rows[0]).toMatchObject({ key: "waiting", kind: "pending", end: t0 + 5_000 })
    expect(trace.rows[0].detail).toBe("Waiting for a worker")
  })

  it("between two attempts, the next one is pending", () => {
    const trace = buildJobTrace(
      job({ state: "delayed", attemptsMade: 1, processedOn: t0 + 1_000, delay: 2_000 }),
      { attempts: 3 },
      t0 + 3_000,
      BULLMQ,
    )
    expect(trace.rows[trace.rows.length - 1]).toMatchObject({ key: "pending", kind: "pending" })
  })

  it("backoff settings", () => {
    expect(backoffMs(500, 3)).toBe(500)
    expect(backoffMs({ type: "fixed", delay: 1_000 }, 2)).toBe(1_000)
    expect(backoffMs({ delay: 1_000 }, 2)).toBe(1_000)
    expect(backoffMs({ type: "exponential", delay: 1_000 }, 3)).toBe(4_000)
    expect(backoffMs({ type: "jitter-custom", delay: 1_000 }, 1)).toBeNull()
    expect(backoffMs(undefined, 1)).toBe(0)
  })
})
