import { beforeEach, describe, expect, it } from "vitest"

import { progressKey, STALL_MS, stallOf, useProgressWatchStore } from "./progressWatch"

beforeEach(() => useProgressWatchStore.setState({ seen: new Map() }))

describe("progress watch", () => {
  it("remembers when the value last changed, not when it was last polled", () => {
    const key = progressKey("c1", "emails", "7")
    const { observe } = useProgressWatchStore.getState()
    observe(key, "10", 1_000)
    observe(key, "10", 5_000)
    expect(useProgressWatchStore.getState().seen.get(key)).toEqual({ value: "10", since: 1_000 })
    observe(key, "20", 9_000)
    expect(useProgressWatchStore.getState().seen.get(key)).toEqual({ value: "20", since: 9_000 })
  })

  it("a stall: progress reported, unchanged for 3 min, lock still renewed", () => {
    const watch = { value: "40", since: 0 }
    expect(stallOf(watch, 12_000, STALL_MS - 1)).toEqual({
      quietMs: STALL_MS - 1,
      stalled: false,
    })
    expect(stallOf(watch, 12_000, STALL_MS).stalled).toBe(true)
    // No lock: the worker is gone: a stalled job, not a slow one; not this signal.
    expect(stallOf(watch, null, STALL_MS).stalled).toBe(false)
    // Never reported progress: nothing to compare.
    expect(stallOf({ value: null, since: 0 }, 12_000, STALL_MS)).toEqual({
      quietMs: null,
      stalled: false,
    })
    expect(stallOf(undefined, 12_000, STALL_MS).stalled).toBe(false)
  })
})
