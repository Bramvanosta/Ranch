import { afterEach, describe, expect, it } from "vitest"

import { type BulkActionKind, useBulkProgressStore } from "./useBulkProgressStore"

afterEach(() => {
  useBulkProgressStore.getState().finish()
})

function perQueueOf(
  entries: Array<{ name: string; action: BulkActionKind; total: number; jobIds: string[] }>,
) {
  const map = new Map<string, { action: BulkActionKind; total: number; jobIds: string[] }>()
  for (const e of entries) map.set(e.name, { action: e.action, total: e.total, jobIds: e.jobIds })
  return map
}

describe("useBulkProgressStore", () => {
  it("start() initializes per-queue maps for a single queue", () => {
    useBulkProgressStore
      .getState()
      .start(
        perQueueOf([
          { name: "emails", action: "retry", total: 5, jobIds: ["1", "2", "3", "4", "5"] },
        ]),
      )
    const s = useBulkProgressStore.getState()
    expect(s.runningActions.size).toBe(1)
    expect(s.runningActions.get("emails")).toEqual({ action: "retry", total: 5 })
    expect(s.perQueueCompleted.get("emails")).toBe(0)
    expect(s.perRow.size).toBe(5)
    expect(s.rowState("1")).toBe("queued")
  })

  it("start() initializes per-queue maps for K queues", () => {
    useBulkProgressStore.getState().start(
      perQueueOf([
        { name: "emails", action: "retry", total: 3, jobIds: ["e1", "e2", "e3"] },
        { name: "reports", action: "retry", total: 2, jobIds: ["r1", "r2"] },
      ]),
    )
    const s = useBulkProgressStore.getState()
    expect(s.runningActions.size).toBe(2)
    expect(s.perRow.size).toBe(5)
    expect(s.aggregateTotal()).toBe(5)
  })

  it("tick() advances a single queue's progress without touching others", () => {
    const store = useBulkProgressStore.getState()
    store.start(
      perQueueOf([
        { name: "emails", action: "retry", total: 10, jobIds: [] },
        { name: "reports", action: "retry", total: 5, jobIds: [] },
      ]),
    )
    store.tick("emails", 7, 1)
    const s = useBulkProgressStore.getState()
    expect(s.perQueueCompleted.get("emails")).toBe(7)
    expect(s.perQueueFailedSoFar.get("emails")).toBe(1)
    expect(s.perQueueCompleted.get("reports")).toBe(0)
    expect(s.perQueueFailedSoFar.get("reports")).toBe(0)
  })

  it("aggregate getters sum across queues", () => {
    const store = useBulkProgressStore.getState()
    store.start(
      perQueueOf([
        { name: "emails", action: "retry", total: 10, jobIds: [] },
        { name: "reports", action: "retry", total: 5, jobIds: [] },
        { name: "images", action: "retry", total: 3, jobIds: [] },
      ]),
    )
    store.tick("emails", 6, 1)
    store.tick("reports", 5, 0)
    store.tick("images", 2, 1)
    const s = useBulkProgressStore.getState()
    expect(s.aggregateCompleted()).toBe(13)
    expect(s.aggregateTotal()).toBe(18)
    expect(s.aggregateFailedSoFar()).toBe(2)
  })

  it("finish() clears every map", () => {
    const store = useBulkProgressStore.getState()
    store.start(perQueueOf([{ name: "emails", action: "retry", total: 5, jobIds: ["1"] }]))
    store.tick("emails", 3, 0)
    store.finish()
    const s = useBulkProgressStore.getState()
    expect(s.runningActions.size).toBe(0)
    expect(s.perQueueCompleted.size).toBe(0)
    expect(s.perQueueFailedSoFar.size).toBe(0)
    expect(s.perRow.size).toBe(0)
    expect(s.aggregateTotal()).toBe(0)
  })

  it("rowState returns null for unknown jobs", () => {
    expect(useBulkProgressStore.getState().rowState("unknown-id")).toBeNull()
  })
})
