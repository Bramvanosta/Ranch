import { describe, expect, it } from "vitest"
import { BULLMQ_CAPABILITIES } from "@/backend/fake/backends"
import type { JobStateInfo } from "@/lib/tauri/bindings"
import {
  describeState,
  isHeadline,
  notStartedStates,
  stateLabel,
  stateList,
  waitingStates,
} from "./jobStates"

const STATES: JobStateInfo[] = [
  { code: "waiting", label: "Waiting", tone: "pending", common: true },
  { code: "parked", label: "Parked here", tone: "warning", common: false },
]

describe("job states", () => {
  it("reads a code as its backend describes it", () => {
    expect(stateLabel("parked", STATES)).toBe("Parked here")
    expect(describeState("waiting", STATES).tone).toBe("pending")
  })

  it("reads a code it is not told about as itself, neutral", () => {
    expect(describeState("held", STATES)).toEqual({
      code: "held",
      label: "held",
      tone: "neutral",
      common: false,
    })
    expect(stateLabel("waiting", [])).toBe("waiting")
  })

  it("always shows the lifecycle's headline states, and a backend's own only when used", () => {
    expect(isHeadline("failed")).toBe(true)
    expect(isHeadline("paused")).toBe(false)
    expect(isHeadline("parked")).toBe(false)
  })

  it("names the states waiting for a worker, and those a drain removes, from the description", () => {
    const bullmq = BULLMQ_CAPABILITIES.jobStates
    expect(stateList(notStartedStates(bullmq, true), bullmq)).toBe(
      "waiting, paused, prioritized and delayed",
    )
    expect(stateList(notStartedStates(bullmq, false), bullmq)).toBe(
      "waiting, paused and prioritized",
    )
    expect(waitingStates(STATES)).toEqual(["waiting"])
    expect(stateList(notStartedStates(STATES, false), STATES)).toBe("waiting and paused")
  })
})
