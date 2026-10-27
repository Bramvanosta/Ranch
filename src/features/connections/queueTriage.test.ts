import { describe, expect, it } from "vitest"

import type { Queue } from "@/lib/tauri/bindings"

import { failedTotal, triageQueues } from "./queueTriage"

function q(name: string, over: Partial<Queue["counters"]> = {}, isPaused = false): Queue {
  return {
    queueRef: { prefix: "bull", name },
    isPaused,
    counters: {
      waiting: 0,
      active: 0,
      completed: 0,
      failed: 0,
      delayed: 0,
      prioritized: 0,
      paused: 0,
      ...over,
    } as Queue["counters"],
  }
}

const names = (xs: { queue?: Queue; queueRef?: Queue["queueRef"] }[]) =>
  xs.map((x) => (x.queue ?? (x as Queue)).queueRef.name)

describe("triageQueues", () => {
  it("puts failing, then paused, then worker-less queues first, by count", () => {
    const queues = [
      q("reports", { waiting: 3 }),
      q("images", { failed: 2 }),
      q("emails", { failed: 37 }),
      q("orders", { waiting: 9 }, true),
      q("enrichment", { waiting: 40 }),
      q("webhooks-outbound-retries-eu-west"),
    ]
    const workers = new Map([
      ["reports", 0],
      ["enrichment", 0],
      ["webhooks-outbound-retries-eu-west", 0],
    ])
    const { problems, rest } = triageQueues(queues, workers)
    expect(problems.map((p) => [p.queue.queueRef.name, p.problem, p.count])).toEqual([
      ["emails", "failed", 37],
      ["images", "failed", 2],
      ["orders", "paused", 9],
      ["enrichment", "noWorker", 40],
      ["reports", "noWorker", 3],
    ])
    // A queue with no worker and nothing waiting is not a problem.
    expect(names(rest)).toEqual(["webhooks-outbound-retries-eu-west"])
  })

  it("never calls a queue worker-less when the worker count is unknown", () => {
    const { problems, rest } = triageQueues([q("reports", { waiting: 3 })], undefined)
    expect(problems).toEqual([])
    expect(names(rest)).toEqual(["reports"])
  })

  it("counts prioritized jobs as waiting", () => {
    const { problems } = triageQueues([q("reports", { prioritized: 4 })], new Map([["reports", 0]]))
    expect(problems[0]?.count).toBe(4)
  })

  it("sorts the quiet queues by name", () => {
    const { rest } = triageQueues([q("orders"), q("emails"), q("images")], new Map())
    expect(names(rest)).toEqual(["emails", "images", "orders"])
  })

  it("adds up failed jobs for a folded row", () => {
    expect(failedTotal([q("a", { failed: 2 }), q("b", { failed: 5 })])).toBe(7)
    expect(failedTotal(undefined)).toBe(0)
  })
})
