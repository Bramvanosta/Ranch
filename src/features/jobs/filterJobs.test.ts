import { describe, expect, it } from "vitest"

import type { Job } from "@/lib/tauri/bindings"

import { filterJobs } from "./filterJobs"

function job(overrides: Partial<Job> = {}): Job {
  return {
    jobRef: { queueId: "bull:emails", queueName: "emails", id: "1" },
    name: "send-email",
    state: "waiting",
    data: "{}",
    opts: "{}",
    attemptsMade: 0,
    timestamp: 0,
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

const FIXTURE: Job[] = [
  job({ name: "send-email", failedReason: null, stacktrace: [] }),
  job({
    name: "send-broken",
    failedReason: "Connection refused",
    stacktrace: ["Error: Connection refused", "    at sendEmail"],
  }),
  job({
    name: "resize-image",
    failedReason: null,
    stacktrace: ["NetworkError: timed out after 30s", "    at fetch"],
  }),
  job({ name: "DAILY-REPORT", failedReason: null, stacktrace: [] }),
]

describe("filterJobs", () => {
  it("returns a fresh array copy when both segments empty", () => {
    const out = filterJobs(FIXTURE, {})
    expect(out).toEqual(FIXTURE)
    expect(out).not.toBe(FIXTURE)
  })

  it("name segment alone, case-insensitive substring", () => {
    expect(filterJobs(FIXTURE, { name: "send" })).toHaveLength(2)
    expect(filterJobs(FIXTURE, { name: "Send" })).toHaveLength(2)
    // Matches DAILY-REPORT too via case-insensitive.
    expect(filterJobs(FIXTURE, { name: "daily" })).toHaveLength(1)
  })

  it("error segment matches against failedReason", () => {
    const out = filterJobs(FIXTURE, { error: "Connection refused" })
    expect(out).toHaveLength(1)
    expect(out[0].name).toBe("send-broken")
  })

  it("error segment matches against stacktrace[0] when failedReason is null", () => {
    const out = filterJobs(FIXTURE, { error: "NetworkError" })
    expect(out).toHaveLength(1)
    expect(out[0].name).toBe("resize-image")
  })

  it("AND semantics when both segments active", () => {
    expect(filterJobs(FIXTURE, { name: "send", error: "Connection" })).toHaveLength(1)
    expect(filterJobs(FIXTURE, { name: "resize", error: "Connection" })).toHaveLength(0)
  })

  it("whitespace-only segments are treated as empty", () => {
    const out = filterJobs(FIXTURE, { name: "   ", error: "\t\n" })
    expect(out).toEqual(FIXTURE)
  })

  it("trims leading/trailing whitespace before matching", () => {
    const out = filterJobs(FIXTURE, { name: "  daily  " })
    expect(out).toHaveLength(1)
    expect(out[0].name).toBe("DAILY-REPORT")
  })

  it("non-failed jobs are excluded when error segment is non-empty", () => {
    const out = filterJobs(FIXTURE, { error: "anything" })
    expect(out.every((j) => j.failedReason !== null || j.stacktrace.length > 0)).toBe(true)
  })
  it("matches the name segment against the job id too (same as the server)", () => {
    const queue = "bull:emails"
    const jobs = [
      job({ jobRef: { queueId: queue, queueName: "emails", id: "4711" }, name: "send-email" }),
      job({ jobRef: { queueId: queue, queueName: "emails", id: "12" }, name: "resize" }),
    ]
    expect(filterJobs(jobs, { name: "471" }).map((j) => j.jobRef.id)).toEqual(["4711"])
  })
})
