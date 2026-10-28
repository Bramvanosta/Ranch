import { describe, expect, it } from "vitest"
import { BULLMQ_CAPABILITIES } from "@/backend/fake/backends"
import type { Queue } from "@/lib/tauri/bindings"
import { failedTotal, triageQueues } from "./queueTriage"

const BULLMQ = BULLMQ_CAPABILITIES.jobStates

function q(name: string, over: Partial<Queue["counts"]> = {}, isPaused = false): Queue {
  return {
    id: `bull:${name}`,
    name,
    isPaused,
    counts: {
      waiting: 0,
      active: 0,
      completed: 0,
      failed: 0,
      delayed: 0,
      prioritized: 0,
      paused: 0,
      ...over,
    } as Queue["counts"],
  }
}

const names = (xs: readonly Queue[]) => xs.map((x) => x.name)

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
      ["bull:reports", 0],
      ["bull:enrichment", 0],
      ["bull:webhooks-outbound-retries-eu-west", 0],
    ])
    const { problems, rest } = triageQueues(queues, workers, BULLMQ)
    expect(problems.map((p) => [p.queue.name, p.problem, p.count])).toEqual([
      ["emails", "failed", 37],
      ["images", "failed", 2],
      ["orders", "paused", 9],
      ["enrichment", "noWorker", 40],
      ["reports", "noWorker", 3],
    ])
    // A queue with no worker and nothing waiting is not a problem.
    expect(names(rest)).toEqual(["webhooks-outbound-retries-eu-west"])
  })

  it("counts a paused queue's held jobs as waiting", () => {
    const { problems } = triageQueues([q("orders", { paused: 100 }, true)], undefined, BULLMQ)
    expect(problems.map((p) => [p.queue.name, p.problem, p.count])).toEqual([
      ["orders", "paused", 100],
    ])
  })

  it("never calls a queue worker-less when the worker count is unknown", () => {
    const { problems, rest } = triageQueues([q("reports", { waiting: 3 })], undefined, BULLMQ)
    expect(problems).toEqual([])
    expect(names(rest)).toEqual(["reports"])
  })

  it("counts prioritized jobs as waiting", () => {
    const { problems } = triageQueues(
      [q("reports", { prioritized: 4 })],
      new Map([["bull:reports", 0]]),
      BULLMQ,
    )
    expect(problems[0]?.count).toBe(4)
  })

  it("sorts the quiet queues by name", () => {
    const { rest } = triageQueues([q("orders"), q("emails"), q("images")], new Map(), BULLMQ)
    expect(names(rest)).toEqual(["emails", "images", "orders"])
  })

  it("adds up failed jobs for a folded row", () => {
    expect(failedTotal([q("a", { failed: 2 }), q("b", { failed: 5 })])).toBe(7)
    expect(failedTotal(undefined)).toBe(0)
  })
})
