import { describe, expect, it } from "vitest"

import { accepts, resolveDrop, type SidebarDndData } from "./sidebarDrop"

const group = (groupId: string): SidebarDndData => ({ type: "group", groupId })
const connection = (connectionId: string, groupId: string): SidebarDndData => ({
  type: "connection",
  connectionId,
  groupId,
})

describe("accepts", () => {
  it("groups land on groups; connections land on groups or connections", () => {
    expect(accepts("group", "group")).toBe(true)
    expect(accepts("group", "connection")).toBe(false)
    expect(accepts("connection", "group")).toBe(true)
    expect(accepts("connection", "connection")).toBe(true)
  })
})

describe("resolveDrop", () => {
  it("reorders groups", () => {
    expect(resolveDrop(group("g1"), group("g2"))).toEqual({
      kind: "reorderGroups",
      activeId: "g1",
      overId: "g2",
    })
    expect(resolveDrop(group("g1"), group("g1"))).toBeNull()
  })

  it("moves a connection to another group, dropped on its header or one of its connections", () => {
    expect(resolveDrop(connection("c1", "g1"), group("g2"))).toEqual({
      kind: "moveToGroup",
      connectionId: "c1",
      groupId: "g2",
    })
    expect(resolveDrop(connection("c1", "g1"), connection("c9", "g2"))).toEqual({
      kind: "moveToGroup",
      connectionId: "c1",
      groupId: "g2",
    })
  })

  it("ignores a drop in the connection's own group (the order is by environment, then name)", () => {
    expect(resolveDrop(connection("c1", "g1"), group("g1"))).toBeNull()
    expect(resolveDrop(connection("c1", "g1"), connection("c2", "g1"))).toBeNull()
    expect(resolveDrop(connection("c1", "g1"), undefined)).toBeNull()
  })
})
