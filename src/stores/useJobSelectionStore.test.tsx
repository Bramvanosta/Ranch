import { afterEach, describe, expect, it } from "vitest"

import type { JobRef, JobState } from "@/lib/tauri/bindings"

import { jobSelectionKey, useJobSelectionStore } from "./useJobSelectionStore"

function entry(queue: string, jobId: string, state: JobState = "failed") {
  const jobRef: JobRef = {
    queue: { prefix: "bull", name: queue },
    id: jobId,
  }
  return { jobRef, state }
}

afterEach(() => {
  useJobSelectionStore.getState().clear()
})

describe("useJobSelectionStore", () => {
  it("jobSelectionKey is stable and unique per (prefix, queue, jobId)", () => {
    const e = entry("emails", "42")
    expect(jobSelectionKey(e)).toBe("bull:emails:42")
  })

  it("toggle adds when absent and removes when present", () => {
    const store = useJobSelectionStore.getState()
    store.toggle(entry("emails", "42"))
    expect(useJobSelectionStore.getState().selection.size).toBe(1)
    store.toggle(entry("emails", "42"))
    expect(useJobSelectionStore.getState().selection.size).toBe(0)
  })

  it("add is idempotent — adding the same key twice keeps size at 1", () => {
    const store = useJobSelectionStore.getState()
    store.add(entry("emails", "42"))
    store.add(entry("emails", "42"))
    expect(useJobSelectionStore.getState().selection.size).toBe(1)
  })

  it("addRange adds multiple entries", () => {
    const store = useJobSelectionStore.getState()
    store.addRange([entry("emails", "1"), entry("emails", "2"), entry("emails", "3")])
    expect(useJobSelectionStore.getState().selection.size).toBe(3)
  })

  it("clear empties the selection", () => {
    const store = useJobSelectionStore.getState()
    store.addRange([entry("emails", "1"), entry("emails", "2")])
    store.clear()
    expect(useJobSelectionStore.getState().selection.size).toBe(0)
  })

  it("selectAll replaces the selection with the given entries", () => {
    const store = useJobSelectionStore.getState()
    store.add(entry("emails", "1"))
    store.selectAll([entry("emails", "2"), entry("emails", "3")])
    expect(useJobSelectionStore.getState().selection.size).toBe(2)
    expect(useJobSelectionStore.getState().isSelected("bull:emails:2")).toBe(true)
    expect(useJobSelectionStore.getState().isSelected("bull:emails:1")).toBe(false)
  })

  it("distinctQueues returns the set of queue names in the selection", () => {
    const store = useJobSelectionStore.getState()
    store.addRange([entry("emails", "1"), entry("emails", "2"), entry("reports", "1")])
    const set = useJobSelectionStore.getState().distinctQueues()
    expect(set.size).toBe(2)
    expect(set.has("emails")).toBe(true)
    expect(set.has("reports")).toBe(true)
  })

  it("isSelected and selectedCount reflect the current state", () => {
    const store = useJobSelectionStore.getState()
    store.add(entry("emails", "42"))
    expect(useJobSelectionStore.getState().isSelected("bull:emails:42")).toBe(true)
    expect(useJobSelectionStore.getState().isSelected("bull:emails:99")).toBe(false)
    expect(useJobSelectionStore.getState().selectedCount()).toBe(1)
  })

  it("remove deletes a single entry by key", () => {
    const store = useJobSelectionStore.getState()
    store.addRange([entry("emails", "1"), entry("emails", "2")])
    store.remove("bull:emails:1")
    expect(useJobSelectionStore.getState().selection.size).toBe(1)
    expect(useJobSelectionStore.getState().isSelected("bull:emails:1")).toBe(false)
  })

  it("removeRange deletes multiple entries by key", () => {
    const store = useJobSelectionStore.getState()
    store.addRange([entry("emails", "1"), entry("emails", "2"), entry("emails", "3")])
    store.removeRange(["bull:emails:1", "bull:emails:2"])
    expect(useJobSelectionStore.getState().selection.size).toBe(1)
    expect(useJobSelectionStore.getState().isSelected("bull:emails:3")).toBe(true)
  })

  it("groupByQueue returns Map<1, N> when K=1", () => {
    const store = useJobSelectionStore.getState()
    store.addRange([entry("emails", "1"), entry("emails", "2"), entry("emails", "3")])
    const groups = useJobSelectionStore.getState().groupByQueue()
    expect(groups.size).toBe(1)
    expect(groups.get("emails")?.length).toBe(3)
  })

  it("groupByQueue returns Map<K, varying counts> when K>1", () => {
    const store = useJobSelectionStore.getState()
    store.addRange([
      entry("emails", "1"),
      entry("emails", "2"),
      entry("reports", "1"),
      entry("images", "1"),
      entry("images", "2"),
    ])
    const groups = useJobSelectionStore.getState().groupByQueue()
    expect(groups.size).toBe(3)
    expect(groups.get("emails")?.length).toBe(2)
    expect(groups.get("reports")?.length).toBe(1)
    expect(groups.get("images")?.length).toBe(2)
  })
})
