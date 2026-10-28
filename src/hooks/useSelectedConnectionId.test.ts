import { describe, expect, it } from "vitest"

import { connectionIdOfPath } from "./useSelectedConnectionId"

describe("connectionIdOfPath", () => {
  it("reads the connection of any screen under it", () => {
    expect(connectionIdOfPath("/connections/c1")).toBe("c1")
    expect(connectionIdOfPath("/connections/c1/queues/emails/42")).toBe("c1")
    expect(connectionIdOfPath("/connections/conn%20a?view=failed")).toBe("conn a")
  })

  it("is null elsewhere", () => {
    expect(connectionIdOfPath("/")).toBeNull()
    expect(connectionIdOfPath("/settings")).toBeNull()
    expect(connectionIdOfPath("/connections")).toBeNull()
  })
})
