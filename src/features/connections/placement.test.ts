import { describe, expect, it } from "vitest"

import type { Environment, Group } from "@/lib/tauri/bindings"

import {
  buildPlacementPayload,
  defaultEnvNameForColor,
  initialPlacement,
  NEW,
  shouldFollowColor,
} from "./placement"

const group = (id: string, name: string): Group => ({
  id,
  name,
  displayOrder: 0,
  collapsed: false,
  connectionCount: 0,
})
const env = (id: string, groupId: string, label: string): Environment => ({
  id,
  groupId,
  groupName: "",
  label,
  name: label,
  colorToken: "env-local",
})

describe("initialPlacement", () => {
  it("proposes a new Default / Local when nothing exists yet", () => {
    expect(initialPlacement([], [])).toEqual({
      group_id: NEW,
      group_name: "Default",
      environment_id: NEW,
      environment_name: "Local",
    })
  })

  it("preselects the first group and its first environment", () => {
    const p = initialPlacement(
      [group("g1", "Lumen"), group("g2", "Acme")],
      [env("e9", "g2", "Prod"), env("e1", "g1", "Local")],
    )
    expect(p.group_id).toBe("g1")
    expect(p.environment_id).toBe("e1")
  })

  it("honours an explicit target, and falls back to a new environment in an empty group", () => {
    const p = initialPlacement([group("g1", "Lumen"), group("g2", "Acme")], [], {
      groupId: "g2",
    })
    expect(p).toMatchObject({ group_id: "g2", environment_id: NEW, environment_name: "Local" })
  })
})

describe("buildPlacementPayload", () => {
  const base = {
    group_id: "g1",
    group_name: "",
    environment_id: "e1",
    environment_name: "",
    env_color: "env-local" as const,
  }

  it("sends existing ids as-is", () => {
    expect(buildPlacementPayload(base)).toEqual({
      group: { kind: "existing", id: "g1" },
      environment: { kind: "existing", id: "e1" },
    })
  })

  it("sends trimmed names for new entries", () => {
    expect(
      buildPlacementPayload({
        ...base,
        group_id: NEW,
        group_name: "  Lumen ",
        environment_id: NEW,
        environment_name: " Staging ",
        env_color: "env-staging",
      }),
    ).toEqual({
      group: { kind: "new", name: "Lumen" },
      environment: { kind: "new", name: "Staging", colorToken: "env-staging" },
    })
  })

  it("never pairs a new group with an existing environment", () => {
    const payload = buildPlacementPayload({
      ...base,
      group_id: NEW,
      group_name: "Lumen",
      environment_name: "Local",
    })
    expect(payload.environment.kind).toBe("new")
  })
})

describe("name follows color only while it is a palette default", () => {
  it("palette names and empty follow; custom names stay", () => {
    expect(shouldFollowColor("")).toBe(true)
    expect(shouldFollowColor("Local")).toBe(true)
    expect(shouldFollowColor("Custom 3")).toBe(true)
    expect(shouldFollowColor("EU West")).toBe(false)
    expect(defaultEnvNameForColor("env-production")).toBe("Production")
  })
})
