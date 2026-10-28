import { describe, expect, it } from "vitest"
import { BULLMQ_CAPABILITIES } from "@/backend/fake/backends"
import type { JobStateInfo } from "@/lib/tauri/bindings"
import { countOf, inLine, knownCount } from "./stateCounts"

const BULLMQ = BULLMQ_CAPABILITIES.jobStates

describe("state counts", () => {
  it("reads a state that was not counted as zero", () => {
    const counts = { waiting: 3, prioritized: 2, paused: 1 }
    expect(countOf(counts, "waiting")).toBe(3)
    expect(countOf(counts, "failed")).toBe(0)
    expect(countOf(undefined, "failed")).toBe(0)
    expect(inLine(counts, BULLMQ)).toBe(5)
    expect(inLine(counts, BULLMQ, true)).toBe(6)
  })

  it("counts in line the states the backend describes as waiting for a worker", () => {
    const counts = { waiting: 3, prioritized: 2, queued: 4, paused: 1 }
    const queued: JobStateInfo[] = [
      { code: "waiting", label: "Waiting", tone: "pending", common: true },
      { code: "queued", label: "Queued", tone: "pending", common: false },
    ]
    expect(inLine(counts, queued)).toBe(7)
    // Before the states are known, the common waiting state.
    expect(inLine(counts, [])).toBe(3)
  })

  it("says when the counts do not know a state", () => {
    expect(knownCount({ failed: 0 }, "failed")).toBe(0)
    expect(knownCount({ failed: 0 }, "delayed")).toBeUndefined()
    expect(knownCount(undefined, "failed")).toBeUndefined()
  })
})
