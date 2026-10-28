import { describe, expect, it } from "vitest"

import { type PaletteItem, paletteSections } from "./paletteItems"

const item = (kind: PaletteItem["kind"], id: string, label = id, hint?: string): PaletteItem => ({
  kind,
  id,
  label,
  hint,
  to: `/${id}`,
})

const allItems = [
  item("queue", "c1:bull:emails", "emails"),
  item("queue", "c2:bull:images", "images"),
  item("connection", "c1", "local", "Lumen Studios / Local"),
  item("group", "go-group-g1", "Go to group: Lumen Studios"),
  item("group", "rename-group-g1", "Rename group: Lumen Studios"),
  item("settings", "go-settings", "Go to Settings"),
  item("action", "command-queue.pause", "Pause"),
]

const base = { allItems, recents: [], currentConnectionId: null, jobId: null, jobItems: [] }

describe("paletteSections", () => {
  it("offers the open connection's queues first, then where to go, on an empty search", () => {
    const sections = paletteSections({ ...base, query: "", currentConnectionId: "c1" })
    expect(sections["Go to"].map((i) => i.id)).toEqual([
      "c1:bull:emails",
      "c1",
      "go-group-g1",
      "go-settings",
    ])
    expect(sections.Actions.map((i) => i.id)).toEqual(["command-queue.pause"])
  })

  it("shows only the jobs found for #id", () => {
    const job = item("job", "job:c1:emails:7")
    const sections = paletteSections({ ...base, query: "#7", jobId: "7", jobItems: [job] })
    expect(sections).toEqual({ Recent: [], "Go to": [], Actions: [], Jobs: [job] })
  })

  it("matches a connection on its group and environment", () => {
    const sections = paletteSections({ ...base, query: "lumen local" })
    expect(sections["Go to"].map((i) => i.id)).toContain("c1")
  })
})
