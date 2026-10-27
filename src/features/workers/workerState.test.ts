import { describe, expect, it } from "vitest"

import { libraryVersions, parseLibrary, workerState } from "./workerState"

describe("worker state", () => {
  it("a blocking pop is waiting for a job; anything else is working", () => {
    expect(workerState({ lastCommand: "bzpopmin" }, false)).toBe("idle")
    expect(workerState({ lastCommand: "BZPOPMIN" }, false)).toBe("idle")
    expect(workerState({ lastCommand: "evalsha" }, false)).toBe("working")
    expect(workerState({ lastCommand: null }, false)).toBe("working")
    expect(workerState({ lastCommand: "bzpopmin" }, true)).toBe("paused")
  })

  it("reads library names and versions", () => {
    expect(parseLibrary("ioredis 5.4.1")).toEqual({ name: "ioredis", version: [5, 4, 1] })
    expect(parseLibrary("ioredis-5.3.2")).toEqual({ name: "ioredis", version: [5, 3, 2] })
    expect(parseLibrary("node-redis 4.6.0")).toEqual({ name: "node-redis", version: [4, 6, 0] })
    expect(parseLibrary(null)).toBeNull()
  })

  it("flags the older versions of a library in use", () => {
    const { distinct, older } = libraryVersions([
      "ioredis 5.4.1",
      "ioredis 5.3.2",
      "ioredis 5.4.1",
      "node-redis 4.6.0",
      null,
    ])
    expect(distinct).toEqual(["ioredis 5.3.2", "ioredis 5.4.1", "node-redis 4.6.0"])
    expect([...older]).toEqual(["ioredis 5.3.2"])
  })
})
