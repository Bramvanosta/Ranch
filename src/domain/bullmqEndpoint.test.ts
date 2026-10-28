import { describe, expect, it } from "vitest"

import type { Connection } from "@/lib/tauri/bindings"

import {
  bullmqConnection,
  endpointLine,
  hostPort,
  readRedisEndpoint,
  redisEndpoint,
  redisUrl,
} from "./bullmqEndpoint"

describe("bullmqEndpoint", () => {
  it("reads a Redis endpoint back, the optional fields filled in", () => {
    const c = { backend: "bullmq-redis", endpoint: { host: "h", port: 6380 } }
    expect(readRedisEndpoint(c)).toEqual(redisEndpoint({ host: "h", port: 6380 }))
    expect(endpointLine(c)).toBe("h:6380 · db 0")
    expect(hostPort(c)).toBe("h:6380")
  })

  it("reads nothing from another backend or an endpoint that is not one", () => {
    const cases: Pick<Connection, "backend" | "endpoint">[] = [
      { backend: "memory", endpoint: { host: "h", port: 1 } },
      { backend: "bullmq-redis", endpoint: {} },
      { backend: "bullmq-redis", endpoint: "redis://h:1" },
      { backend: "bullmq-redis", endpoint: null },
    ]
    for (const c of cases) {
      expect(readRedisEndpoint(c)).toBeNull()
      expect(hostPort(c)).toBeNull()
      expect(redisUrl(c)).toBeNull()
    }
  })

  it("writes a URL with the user and never a password", () => {
    const c = bullmqConnection(
      redisEndpoint({ host: "redis.lumen.example", db: 2, username: "app user", tls: true }),
    )
    expect(c.backend).toBe("bullmq-redis")
    expect(redisUrl(c)).toBe("rediss://app%20user@redis.lumen.example:6379/2")
    expect(redisUrl(bullmqConnection(redisEndpoint({ host: "h" })))).toBe("redis://h:6379/0")
  })
})
