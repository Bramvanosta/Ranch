import { describe, expect, it } from "vitest"
import { BULLMQ_CAPABILITIES } from "@/backend/fake/backends"
import { bullmqConnection, redisEndpoint } from "@/domain/bullmqEndpoint"

import type { GroupNode } from "@/features/connections"
import type { Connection, Queue } from "@/lib/tauri/bindings"

import {
  compactCount,
  countConnections,
  filterTree,
  matchesConnection,
  summarizeQueues,
} from "./homeFilter"

const conn = (over: Partial<Connection>): Connection =>
  ({
    id: "c1",
    name: "Queues",
    ...bullmqConnection(redisEndpoint({ host: "redis.internal" })),
    envLabel: "Staging",
    envName: "staging",
    groupName: "Lumen",
    ...over,
  }) as Connection

const withConnections = (g: Omit<GroupNode, "connections">): GroupNode => ({
  ...g,
  connections: g.envs.flatMap((e) => e.connections),
})

const tree: GroupNode[] = (
  [
    {
      id: "g1",
      name: "Lumen",
      collapsed: false,
      envs: [
        {
          id: "e1",
          name: "staging",
          label: "Staging",
          colorToken: "env-staging",
          connections: [conn({ id: "a", name: "Queues" }), conn({ id: "b", name: "Cache" })],
        },
        {
          id: "e2",
          name: "production",
          label: "Production",
          colorToken: "env-production",
          connections: [
            conn({ id: "c", name: "Queues", envLabel: "Production", envName: "production" }),
          ],
        },
      ],
    },
    {
      id: "g2",
      name: "Side project",
      collapsed: true,
      envs: [
        {
          id: "e3",
          name: "local",
          label: "Local",
          colorToken: "env-local",
          connections: [
            conn({
              id: "d",
              name: "dev",
              ...bullmqConnection(redisEndpoint({ host: "127.0.0.1", port: 6390 })),
              groupName: "Side project",
            }),
          ],
        },
      ],
    },
  ] as Omit<GroupNode, "connections">[]
).map(withConnections)

describe("search", () => {
  it("matches name, host:port, environment and group, every word", () => {
    const c = conn({})
    expect(matchesConnection(c, "")).toBe(true)
    expect(matchesConnection(c, "QUEUES")).toBe(true)
    expect(matchesConnection(c, "redis.internal:6379")).toBe(true)
    expect(matchesConnection(c, "lumen staging")).toBe(true)
    expect(matchesConnection(c, "lumen production")).toBe(false)
  })

  it("narrows the tree and drops what became empty", () => {
    const prod = filterTree(tree, "production")
    expect(prod.map((g) => g.name)).toEqual(["Lumen"])
    expect(prod[0].envs.map((e) => e.label)).toEqual(["Production"])
    expect(countConnections(prod)).toBe(1)

    expect(filterTree(tree, "6390").map((g) => g.name)).toEqual(["Side project"])
    expect(filterTree(tree, "nothing-like-this")).toEqual([])
    expect(countConnections(filterTree(tree, "  "))).toBe(4)
  })
})

describe("health", () => {
  const queue = (over: Partial<Queue["counts"]>, isPaused = false): Queue =>
    ({
      id: "bull:q",
      name: "q",
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
      },
    }) as Queue

  it("sums the queues; parked and prioritized jobs count as waiting", () => {
    expect(
      summarizeQueues(
        [
          queue({ waiting: 10, active: 2, failed: 3 }),
          queue({ paused: 40, prioritized: 5, delayed: 7 }, true),
        ],
        BULLMQ_CAPABILITIES.jobStates,
      ),
    ).toEqual({ queues: 2, pausedQueues: 1, waiting: 55, active: 2, failed: 3, delayed: 7 })
    expect(summarizeQueues([], [])).toMatchObject({ queues: 0, failed: 0 })
  })

  it("compacts big numbers", () => {
    expect([0, 999, 1_000, 1_234, 12_345, 999_499, 1_250_000].map(compactCount)).toEqual([
      "0",
      "999",
      "1k",
      "1.2k",
      "12k",
      "999k",
      "1.3M",
    ])
  })
})
