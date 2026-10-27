import { describe, expect, it } from "vitest"

import { altArrowDelta, moveItemInList } from "./altArrowReorder"

describe("moveItemInList", () => {
  it("moves an item up by one", () => {
    expect(moveItemInList(["a", "b", "c"], 1, -1)).toEqual(["b", "a", "c"])
  })

  it("moves an item down by one", () => {
    expect(moveItemInList(["a", "b", "c"], 1, 1)).toEqual(["a", "c", "b"])
  })

  it("clamps at the top — moving index 0 up is a no-op", () => {
    expect(moveItemInList(["a", "b", "c"], 0, -1)).toEqual(["a", "b", "c"])
  })

  it("clamps at the bottom — moving last index down is a no-op", () => {
    expect(moveItemInList(["a", "b", "c"], 2, 1)).toEqual(["a", "b", "c"])
  })

  it("returns a new array (immutability)", () => {
    const input = ["a", "b", "c"]
    const output = moveItemInList(input, 0, 1)
    expect(output).not.toBe(input)
    expect(input).toEqual(["a", "b", "c"])
  })

  it("handles a multi-position move", () => {
    expect(moveItemInList(["a", "b", "c", "d", "e"], 1, 2)).toEqual(["a", "c", "d", "b", "e"])
  })
})

describe("altArrowDelta", () => {
  function evt(opts: { altKey?: boolean; key?: string }): KeyboardEvent {
    return {
      altKey: opts.altKey ?? false,
      key: opts.key ?? "",
    } as KeyboardEvent
  }

  it("returns -1 for Alt+ArrowUp", () => {
    expect(altArrowDelta(evt({ altKey: true, key: "ArrowUp" }))).toBe(-1)
  })

  it("returns 1 for Alt+ArrowDown", () => {
    expect(altArrowDelta(evt({ altKey: true, key: "ArrowDown" }))).toBe(1)
  })

  it("returns null when alt is not held", () => {
    expect(altArrowDelta(evt({ altKey: false, key: "ArrowUp" }))).toBeNull()
    expect(altArrowDelta(evt({ altKey: false, key: "ArrowDown" }))).toBeNull()
  })

  it("returns null for non-arrow keys", () => {
    expect(altArrowDelta(evt({ altKey: true, key: "ArrowLeft" }))).toBeNull()
    expect(altArrowDelta(evt({ altKey: true, key: "Enter" }))).toBeNull()
  })
})
