import { describe, expect, it } from "vitest"

import {
  type BarState,
  bulkBarReducer,
  type CompleteResult,
  cancelledSummary,
  failedSummary,
} from "./bulkActionState"

const selecting: BarState = { kind: "selecting" }

describe("bulkBarReducer", () => {
  it("goes selecting → preview → executing → complete", () => {
    const preview = bulkBarReducer(selecting, { type: "preview", action: "retry" })
    expect(preview).toEqual({ kind: "preview", action: "retry" })
    const executing = bulkBarReducer(preview, { type: "execute", action: "retry" })
    expect(executing).toEqual({ kind: "executing", action: "retry" })
    const result: CompleteResult = failedSummary("retry", 3, "locked")
    expect(bulkBarReducer(executing, { type: "complete", action: "retry", result })).toEqual({
      kind: "complete",
      action: "retry",
      result,
    })
  })

  it("previews only from selecting, and executes only from a preview", () => {
    const executing: BarState = { kind: "executing", action: "remove" }
    expect(bulkBarReducer(executing, { type: "preview", action: "retry" })).toBe(executing)
    expect(bulkBarReducer(selecting, { type: "execute", action: "retry" })).toBe(selecting)
  })

  it("goes back to selecting on cancel and on reset", () => {
    const preview: BarState = { kind: "preview", action: "promote" }
    expect(bulkBarReducer(preview, { type: "cancel" })).toEqual(selecting)
    expect(bulkBarReducer(preview, { type: "reset" })).toEqual(selecting)
  })
})

describe("summaries", () => {
  it("reports a call that failed as a whole as one failure with its reason", () => {
    expect(failedSummary("remove", 4, "Connection is locked")).toEqual({
      mode: "single",
      result: {
        action: "remove",
        total: 4,
        succeeded: 0,
        failed: [{ jobId: "", reason: "Connection is locked" }],
        cancelled: false,
      },
    })
  })

  it("counts what succeeded before the user stopped waiting", () => {
    const summary = cancelledSummary("retry", { total: 10, completed: 6, failedSoFar: 1 })
    expect(summary.result).toMatchObject({ total: 10, succeeded: 5, cancelled: true })
    expect(
      cancelledSummary("retry", { total: 2, completed: 0, failedSoFar: 1 }).result,
    ).toMatchObject({ succeeded: 0 })
  })
})
