import { describe, expect, it } from "vitest"
import { BULLMQ_CAPABILITIES } from "@/backend/fake/backends"
import type { StateCounts } from "@/lib/tauri/bindings"
import { jobColumns } from "./jobColumns"
import { visibleStateTabs } from "./jobStates"

const ids = (state: Parameters<typeof jobColumns>[0]["state"], showQueue = false) =>
  jobColumns({ state, showQueue }).map((c) => c.id)

describe("job columns per state", () => {
  it("failed: job, ID, last error, tries, failed, ran", () => {
    expect(ids("failed")).toEqual([
      "select",
      "job",
      "id",
      "error",
      "tries",
      "failedAt",
      "duration",
      "actions",
    ])
  })

  it("waiting: job, ID, added, priority, delay", () => {
    expect(ids("waiting")).toEqual(["select", "job", "id", "added", "priority", "delay", "actions"])
    expect(ids("prioritized")).toEqual(ids("waiting"))
  })

  it("active: progress, running against the usual, worker, attempt", () => {
    expect(ids("active")).toEqual([
      "select",
      "job",
      "id",
      "progress",
      "running",
      "worker",
      "attempt",
      "actions",
    ])
  })

  it("completed: finished, took, tries; delayed: runs in first", () => {
    expect(ids("completed")).toContain("finishedAt")
    expect(ids("delayed").indexOf("runsIn")).toBe(3)
  })

  it("the all-queues scope adds the queue after the ID", () => {
    expect(ids("failed", true).indexOf("queue")).toBe(3)
  })

  it("no column of any state is set to clip: text wraps, the row grows", () => {
    for (const state of ["failed", "waiting", "active", "delayed", "completed"] as const) {
      for (const c of jobColumns({ state })) expect(c.track).not.toMatch(/fit-content/)
    }
  })
})

describe("state tabs", () => {
  const counters = (over: Partial<StateCounts> = {}) =>
    ({
      waiting: 1,
      active: 0,
      completed: 0,
      failed: 0,
      delayed: 0,
      prioritized: 0,
      paused: 0,
      ...over,
    }) as StateCounts

  it("the five headline states always, Prioritized and Paused when they hold jobs or are open", () => {
    const tabs = (counts: StateCounts, open: string) =>
      visibleStateTabs(BULLMQ_CAPABILITIES.jobStates, counts, open).map((t) => t.code)
    expect(tabs(counters(), "waiting")).toEqual([
      "waiting",
      "active",
      "delayed",
      "failed",
      "completed",
    ])
    expect(tabs(counters({ prioritized: 2 }), "waiting")).toContain("prioritized")
    expect(tabs(counters(), "paused")).toContain("paused")
  })
})
