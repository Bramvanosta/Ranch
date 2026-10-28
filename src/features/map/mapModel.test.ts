import { describe, expect, it } from "vitest"
import { BULLMQ_CAPABILITIES } from "@/backend/fake/backends"
import type { Queue, QueueWorker, Throughput, Topology } from "@/lib/tauri/bindings"
import { buildMap, failureTone, NO_WORKER, poolOf } from "./mapModel"

const queue = (name: string, over: Partial<Queue["counts"]> = {}, paused = false) =>
  ({
    id: `bull:${name}`,
    name,
    isPaused: paused,
    counts: {
      waiting: 0,
      active: 0,
      prioritized: 0,
      failed: 0,
      completed: 0,
      delayed: 0,
      ...over,
    },
  }) as unknown as Queue

const worker = (name: string | null, addr: string) =>
  ({ id: addr, name, addr }) as unknown as QueueWorker

const throughput = {
  range: "1h",
  bucketMs: 600_000,
  bucketStartsMs: [0, 1, 2, 3, 4, 5],
  queues: [
    {
      queueId: "bull:emails",
      queueName: "emails",
      metricsOff: false,
      completed: [100, 100, 100, 100, 100, 50],
      failed: [5, 5, 5, 5, 5, 5],
    },
    {
      queueId: "bull:orders",
      queueName: "orders",
      metricsOff: false,
      completed: [6, 6, 6, 6, 6, 6],
      failed: [0, 0, 0, 0, 0, 0],
    },
  ],
} as unknown as Throughput

const topology = {
  links: [
    {
      parent: "bull:orders",
      child: "bull:emails",
      jobs: 4,
    },
  ],
  queues: [
    {
      queueId: "bull:emails",
      addedPerMinute: 9.5,
      addedCapped: false,
      oldestWaitingMs: 1000,
    },
  ],
} as unknown as Topology

describe("map model", () => {
  it("puts queues by throughput, pools by the queues they consume, and a No worker node", () => {
    const m = buildMap({
      queues: [queue("idle"), queue("orders"), queue("emails"), queue("reports", { waiting: 3 })],
      throughput,
      workers: new Map([
        ["emails", [worker("mailer", "10.0.0.1:5000"), worker("mailer", "10.0.0.2:5000")]],
        ["orders", [worker(null, "10.0.0.9:6000")]],
      ]),
      topology,
      onlyActive: true,
      states: BULLMQ_CAPABILITIES.jobStates,
    })
    expect(m.queues.map((q) => q.name)).toEqual(["emails", "orders", "reports"])
    expect(m.hidden).toBe(1)
    expect(m.pools.map((p) => p.name)).toEqual(["mailer", "orders workers"])
    expect(m.noWorker).toBe(true)
    const emails = m.queues[0]
    expect(emails.outPerMin).toBeCloseTo(580 / 60)
    expect(emails.failureRate).toBeCloseTo(30 / 580)
    expect(emails.inPerMin).toBe(9.5)
    expect(m.edges.find((e) => e.queue === "reports")?.to).toBe(NO_WORKER)
    expect(m.flows).toEqual([{ parent: "orders", child: "emails", jobs: 4 }])
  })

  it("splits unnamed workers by queue, even from one address (a container gateway)", () => {
    const m = buildMap({
      queues: [queue("orders", { waiting: 1 }), queue("emails", { waiting: 1 })],
      throughput,
      workers: new Map([
        ["orders", [worker(null, "192.168.164.1:5000"), worker(null, "192.168.164.1:5001")]],
        ["emails", [worker(null, "192.168.164.1:5002")]],
      ]),
      topology,
      onlyActive: false,
      states: BULLMQ_CAPABILITIES.jobStates,
    })
    expect(m.pools.map((p) => [p.name, p.workers]).sort()).toEqual([
      ["emails workers", 1],
      ["orders workers", 2],
    ])
  })

  it("shows every queue on request, idle ones dashed", () => {
    const m = buildMap({
      queues: [queue("idle"), queue("paused", {}, true)],
      throughput: undefined,
      workers: new Map([["paused", [worker("w", "1.1.1.1:1")]]]),
      topology: undefined,
      onlyActive: false,
      states: BULLMQ_CAPABILITIES.jobStates,
    })
    expect(m.queues).toHaveLength(2)
    expect(m.edges.every((e) => e.dashed)).toBe(true)
  })

  it("names tones and pools", () => {
    expect(failureTone(0.01)).toBe("ok")
    expect(failureTone(0.05)).toBe("warn")
    expect(failureTone(0.2)).toBe("danger")
    expect(poolOf(worker("api", "a:1"), "orders").id).toBe("name:api")
    expect(poolOf(worker(null, "a:1"), "orders")).toEqual({
      id: "queue:orders",
      name: "orders workers",
    })
  })
})
