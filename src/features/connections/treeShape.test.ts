import { describe, expect, it } from "vitest"
import { bullmqConnection, redisEndpoint } from "@/domain/bullmqEndpoint"

import type { Connection } from "@/lib/tauri/bindings"

import { buildSidebarTree, groupConnectionsByHierarchy } from "./treeShape"

function conn(overrides: Partial<Connection>): Connection {
  return {
    id: "c1",
    environmentId: "e1",
    groupId: "g1",
    name: "conn",
    ...bullmqConnection(redisEndpoint({ host: "h" })),
    envColor: "env-local",
    envName: "Local",
    envLabel: "Environment",
    groupName: "Default",
    createdAt: "2026-05-03T00:00:00Z",
    mcpReadonly: null,
    sidebarCollapsed: false,
    ...overrides,
  }
}

describe("groupConnectionsByHierarchy", () => {
  it("returns an empty array for empty input", () => {
    expect(groupConnectionsByHierarchy([])).toEqual([])
  })

  it("groups two connections under the same env", () => {
    const result = groupConnectionsByHierarchy([
      conn({ id: "a", name: "A" }),
      conn({ id: "b", name: "B" }),
    ])
    expect(result).toHaveLength(1)
    expect(result[0].envs).toHaveLength(1)
    expect(result[0].envs[0].connections.map((c) => c.id)).toEqual(["a", "b"])
  })

  it("splits connections into separate envs when colors differ", () => {
    const result = groupConnectionsByHierarchy([
      conn({ id: "a", environmentId: "e1", envColor: "env-local", envName: "Local" }),
      conn({
        id: "b",
        environmentId: "e2",
        envColor: "env-production",
        envName: "Prod",
      }),
    ])
    expect(result).toHaveLength(1)
    expect(result[0].envs).toHaveLength(2)
    expect(result[0].envs[0].id).toBe("e1")
    expect(result[0].envs[1].id).toBe("e2")
  })

  it("splits into separate groups when group ids differ", () => {
    const result = groupConnectionsByHierarchy([
      conn({ id: "a", groupId: "g1", groupName: "G1" }),
      conn({ id: "b", groupId: "g2", groupName: "G2" }),
    ])
    expect(result).toHaveLength(2)
    expect(result.map((g) => g.id)).toEqual(["g1", "g2"])
  })

  it("preserves backend ordering (insertion order via Map)", () => {
    // Backend orders by group.display_order, env.display_order, conn.created_at.
    // We trust the input order verbatim.
    const result = groupConnectionsByHierarchy([
      conn({ id: "third", name: "third" }),
      conn({ id: "first", name: "first" }),
      conn({ id: "second", name: "second" }),
    ])
    expect(result[0].envs[0].connections.map((c) => c.name)).toEqual(["third", "first", "second"])
  })
})

describe("buildSidebarTree", () => {
  const group = (id: string, name: string, collapsed = false) => ({
    id,
    name,
    displayOrder: 0,
    collapsed,
    connectionCount: 0,
  })

  it("lists empty groups and follows the order of list_groups", () => {
    const tree = buildSidebarTree(
      [group("g2", "Acme"), group("g1", "Lumen", true)],
      [conn({ id: "c1", groupId: "g1", groupName: "Lumen" })],
    )
    expect(tree.map((g) => g.name)).toEqual(["Acme", "Lumen"])
    expect(tree[0].envs).toEqual([])
    expect(tree[1].collapsed).toBe(true)
    expect(tree[1].envs[0].connections.map((c) => c.id)).toEqual(["c1"])
  })

  it("prefers the name from list_groups over the one denormalised on connections", () => {
    const tree = buildSidebarTree(
      [group("g1", "Renamed")],
      [conn({ id: "c1", groupId: "g1", groupName: "Default" })],
    )
    expect(tree[0].name).toBe("Renamed")
  })

  it("keeps a group that only the connections query knows about", () => {
    const tree = buildSidebarTree([], [conn({ id: "c1", groupId: "g9", groupName: "Late" })])
    expect(tree.map((g) => g.id)).toEqual(["g9"])
  })
})

describe("buildSidebarTree: environments", () => {
  const g = { id: "g1", name: "Lumen", displayOrder: 0, collapsed: false, connectionCount: 1 }
  const e = (id: string, label: string, colorToken = "env-local") => ({
    id,
    groupId: "g1",
    groupName: "Lumen",
    label,
    name: label,
    colorToken,
  })

  it("shows environments left empty, in list_environments order, with fresh label + color", () => {
    const tree = buildSidebarTree(
      [g],
      [
        conn({
          id: "c1",
          groupId: "g1",
          environmentId: "e2",
          envLabel: "stale",
          envColor: "env-local",
        }),
      ],
      [e("e1", "Local"), e("e2", "Production", "env-production")],
    )
    expect(tree[0].envs.map((x) => x.label)).toEqual(["Local", "Production"])
    expect(tree[0].envs[0].connections).toEqual([])
    expect(tree[0].envs[1].colorToken).toBe("env-production")
    expect(tree[0].envs[1].connections.map((c) => c.id)).toEqual(["c1"])
  })

  it("keeps an environment that only the connections query knows about", () => {
    const tree = buildSidebarTree([g], [conn({ id: "c1", groupId: "g1", environmentId: "e9" })], [])
    expect(tree[0].envs.map((x) => x.id)).toEqual(["e9"])
  })
})
