import type { BullmqRedisEndpoint, Connection, JsonValue } from "@/lib/tauri/bindings"

/**
 * The endpoint of a BullMQ-on-Redis connection: the only module that knows
 * its shape. A connection carries its endpoint as JSON, owned by its
 * backend; everything that shows a host or port, builds a URL or fills the
 * connection form reads it through here. Passwords are never part of it.
 */

/** The kind a BullMQ-on-Redis connection names. */
export const BULLMQ_REDIS = "bullmq-redis"

type WithEndpoint = Pick<Connection, "backend" | "endpoint">

/** A Redis endpoint, from the fields given and the defaults (6379, db 0, no TLS). */
export function redisEndpoint(
  fields: Partial<BullmqRedisEndpoint> & Pick<BullmqRedisEndpoint, "host">,
): BullmqRedisEndpoint {
  return {
    port: 6379,
    db: 0,
    username: null,
    tls: false,
    tlsCustomCa: null,
    sshTunnel: null,
    ...fields,
  }
}

/** The endpoint as a connection carries it. */
export function endpointJson(endpoint: BullmqRedisEndpoint): JsonValue {
  return endpoint
}

/** A BullMQ-on-Redis connection's endpoint: `{ backend, endpoint }` for a save or a test. */
export function bullmqConnection(endpoint: BullmqRedisEndpoint): WithEndpoint {
  return { backend: BULLMQ_REDIS, endpoint: endpointJson(endpoint) }
}

function isObject(v: JsonValue | undefined): v is { [key in string]: JsonValue } {
  return typeof v === "object" && v !== null && !Array.isArray(v)
}

/**
 * The connection's Redis endpoint, or `null` for another kind of backend or
 * an endpoint that is not one (an unreadable one reads as `{}`).
 */
export function readRedisEndpoint(c: WithEndpoint): BullmqRedisEndpoint | null {
  const e = c.endpoint
  if (c.backend !== BULLMQ_REDIS || !isObject(e)) return null
  if (typeof e.host !== "string" || typeof e.port !== "number") return null
  return redisEndpoint({
    host: e.host,
    port: e.port,
    db: typeof e.db === "number" ? e.db : 0,
    username: typeof e.username === "string" ? e.username : null,
    tls: e.tls === true,
    tlsCustomCa: typeof e.tlsCustomCa === "string" ? e.tlsCustomCa : null,
    sshTunnel: isObject(e.sshTunnel)
      ? (e.sshTunnel as unknown as BullmqRedisEndpoint["sshTunnel"])
      : null,
  })
}

/** `host:port`, or `null` when the endpoint is not a Redis one. */
export function hostPort(c: WithEndpoint): string | null {
  const e = readRedisEndpoint(c)
  return e ? `${e.host}:${e.port}` : null
}

/** `host:port · db 2`, or `null` when the endpoint is not a Redis one. */
export function endpointLine(c: WithEndpoint): string | null {
  const e = readRedisEndpoint(c)
  return e ? `${e.host}:${e.port} · db ${e.db}` : null
}

/** The connection as a Redis URL, never with its password (that stays in the keychain). */
export function redisUrl(c: WithEndpoint): string | null {
  const e = readRedisEndpoint(c)
  if (!e) return null
  const user = e.username ? `${encodeURIComponent(e.username)}@` : ""
  return `redis${e.tls ? "s" : ""}://${user}${e.host}:${e.port}/${e.db}`
}
