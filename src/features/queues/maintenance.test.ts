import { describe, expect, it } from "vitest"

import { describePreview, describeResult, maintenanceCopy, OLDER_THAN_OPTIONS } from "./maintenance"

describe("maintenance copy", () => {
  it("marks the operations that lose data", () => {
    expect(maintenanceCopy("drain", "emails").destructive).toBe(true)
    expect(maintenanceCopy("clean", "emails").destructive).toBe(true)
    expect(maintenanceCopy("retryAllFailed", "emails").destructive).toBe(false)
    expect(maintenanceCopy("drain", "emails").title).toBe("Drain emails")
  })

  it("offers 'all of them' first, then growing ages", () => {
    expect(OLDER_THAN_OPTIONS[0]).toEqual({ label: "All of them", ms: 0 })
    const ages = OLDER_THAN_OPTIONS.map((o) => o.ms)
    expect([...ages].sort((a, b) => a - b)).toEqual(ages)
  })
})

describe("describePreview", () => {
  it("states the affected count before anything runs", () => {
    expect(
      describePreview(
        { kind: "drain", includeDelayed: true },
        { affected: 1234, sparedSchedulerJobs: 0 },
      ),
    ).toBe("1,234 jobs will be removed.")
    expect(
      describePreview({ kind: "retryAllFailed" }, { affected: 1, sparedSchedulerJobs: 0 }),
    ).toBe("1 job will be retried.")
  })

  it("says what a drain leaves in place", () => {
    expect(
      describePreview(
        { kind: "drain", includeDelayed: true },
        { affected: 7, sparedSchedulerJobs: 2 },
      ),
    ).toBe("7 jobs will be removed. The next run of 2 schedulers is left in place.")
  })

  it("has a sentence for 'nothing to do'", () => {
    const none = { affected: 0, sparedSchedulerJobs: 0 }
    expect(describePreview({ kind: "drain", includeDelayed: true }, none)).toBe("Nothing to drain.")
    expect(describePreview({ kind: "clean", state: "failed", olderThanMs: 0 }, none)).toBe(
      "No job matches.",
    )
    expect(describePreview({ kind: "retryAllFailed" }, none)).toBe("No failed job to retry.")
  })
})

describe("describeResult", () => {
  it("summarises what happened, what was skipped and an interruption", () => {
    expect(describeResult({ action: "drain", affected: 6, skipped: 0, cancelled: false })).toBe(
      "6 jobs removed",
    )
    expect(describeResult({ action: "clean", affected: 1200, skipped: 1, cancelled: false })).toBe(
      "1,200 jobs removed · 1 left in place (flows, scheduler runs)",
    )
    expect(describeResult({ action: "retry", affected: 40, skipped: 2, cancelled: true })).toBe(
      "40 jobs retried · 2 no longer existed · interrupted before the end",
    )
  })
})
