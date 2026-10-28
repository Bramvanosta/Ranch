import { describe, expect, it } from "vitest"

import type { Connection, Queue } from "@/lib/tauri/bindings"

import { filterQueues, highlightParts, looksLikeJobId } from "./sidebarFilter"

const conn = (id: string, groupId: string) => ({ id, groupId, name: id }) as Connection
const queue = (name: string) => ({ id: `bull:${name}`, name }) as Queue

describe("filterQueues", () => {
  const groups = [
    { id: "g1", name: "Lumen" },
    { id: "g2", name: "Side project" },
  ]
  const connections = [conn("local", "g1"), conn("prod", "g1"), conn("play", "g2")]
  const queues: Record<string, Queue[]> = {
    local: [queue("billing"), queue("emails"), queue("billing-retries")],
    prod: [queue("Billing"), queue("images")],
    play: [queue("emails")],
  }

  it("keeps matching queues under their connection and group, and counts them", () => {
    const out = filterQueues("bill", groups, connections, (id) => queues[id])
    expect(out.queueCount).toBe(3)
    expect(out.connectionCount).toBe(2)
    expect(out.groups.map((g) => g.groupName)).toEqual(["Lumen"])
    expect(out.groups[0]?.connections.map((c) => c.queues.map((q) => q.name))).toEqual([
      ["billing", "billing-retries"],
      ["Billing"],
    ])
  })

  it("skips connections whose queues are not known yet", () => {
    const out = filterQueues("emails", groups, connections, (id) =>
      id === "play" ? undefined : queues[id],
    )
    expect(out.connectionCount).toBe(1)
  })

  it("tells a job id from a queue name", () => {
    expect(looksLikeJobId("#1211")).toBe(true)
    expect(looksLikeJobId(" 1211 ")).toBe(true)
    expect(looksLikeJobId("bill")).toBe(false)
    expect(looksLikeJobId("enrich-film-1")).toBe(false)
  })

  it("splits a name around the match, whatever the case", () => {
    expect(highlightParts("Billing-retries", "bill")).toEqual(["", "Bill", "ing-retries"])
    expect(highlightParts("emails", "bill")).toEqual(["emails", "", ""])
  })
})
