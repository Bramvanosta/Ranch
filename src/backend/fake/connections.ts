import { BULLMQ_REDIS, readRedisEndpoint } from "@/domain/bullmqEndpoint"
import type { Connection, Environment, Group, JsonValue } from "@/lib/tauri/bindings"
import { BULLMQ_REDIS_INFO } from "./backends"
import type { FakeHandlers } from "./handlers"
import { between } from "./seed"
import { a, audit, connectionOf, isLocked, UNREACHABLE, world } from "./world"

/** The host a test or save names, from its endpoint. */
const hostOf = (args: Record<string, unknown>) =>
  readRedisEndpoint({
    backend: String(args.backend ?? ""),
    endpoint: (args.endpoint ?? null) as JsonValue,
  })?.host

/** Connections, groups, environments and the session lock. */
export const connectionsHandlers = {
  // connections & hierarchy
  list_connections: () => world.connections,
  list_backends: () => [BULLMQ_REDIS_INFO],
  list_groups: () =>
    world.groups.map((g) => ({
      ...g,
      connectionCount: world.connections.filter((c) => c.groupId === g.id).length,
    })),
  list_environments: () => world.environments,
  ping_connection: ({ connectionId }) =>
    connectionId === UNREACHABLE
      ? {
          success: false,
          latencyMs: null,
          error: { kind: "cannotReach", reason: "Connection refused (os error 61)" },
        }
      : { success: true, latencyMs: connectionId === "c-local" ? 1 : between(18, 90), error: null },
  test_connection: (args) =>
    hostOf(a(args)) === "fail"
      ? {
          success: false,
          redisVersion: null,
          error: { kind: "cannotReach", reason: "Connection refused" },
          sshHostKeyPinned: null,
        }
      : {
          success: true,
          redisVersion: "7.2.4",
          error: null,
          sshHostKeyPinned: null,
          probe: {
            latencyMs: 3,
            queuesFound: 12,
            queueSample: ["bull:emails", "bull:exports", "bull:image-resize", "bull:webhooks"],
            scanCapped: false,
            schedulers: 5,
            workers: 8,
          },
        },
  parse_redis_url: ({ url }) => {
    const u = new URL(String(url))
    return {
      host: u.hostname,
      port: Number(u.port || 6379),
      db: Number(u.pathname.slice(1) || 0),
      username: u.username || null,
      password: u.password || null,
      tls: u.protocol === "rediss:",
    }
  },
  add_connection: (args) => {
    const input = a(args)
    const env = world.environments[0] as Environment
    const created: Connection = {
      ...(world.connections[0] as Connection),
      id: `c-${Date.now()}`,
      name: String(input.name ?? "New connection"),
      backend: String(input.backend ?? BULLMQ_REDIS),
      endpoint: (input.endpoint ?? {}) as JsonValue,
      environmentId: env.id,
      groupId: env.groupId,
      groupName: env.groupName,
      envColor: env.colorToken,
      envName: env.name,
      envLabel: env.label,
      createdAt: new Date().toISOString(),
      sidebarCollapsed: false,
    }
    world.connections.push(created)
    world.queues.set(created.id, [])
    return created
  },
  update_connection: (args) => {
    const input = a(args)
    return Object.assign(connectionOf(input.id), {
      name: input.name ?? undefined,
      ...(input.endpoint ? { endpoint: input.endpoint as JsonValue } : {}),
    })
  },
  duplicate_connection: (args) => {
    const source = connectionOf(a(args).id)
    const copy = { ...source, id: `c-${Date.now()}`, name: `${source.name} (copy)` }
    world.connections.push(copy)
    world.queues.set(copy.id, world.queues.get(source.id) ?? [])
    return copy
  },
  remove_connection: ({ id }) => {
    world.connections = world.connections.filter((c) => c.id !== id)
    return null
  },
  move_connection: (args) => {
    const c = connectionOf(a(args).connectionId)
    const env = world.environments.find((e) => e.id === a(args).environmentId)
    if (env)
      Object.assign(c, {
        environmentId: env.id,
        groupId: env.groupId,
        groupName: env.groupName,
        envColor: env.colorToken,
        envName: env.name,
        envLabel: env.label,
      })
    return { newlyLocked: env?.colorToken === "env-production" }
  },
  rename_environment: () => null,
  change_environment_color: () => null,
  create_environment: (args) => {
    const input = a(args)
    const group = world.groups.find((g) => g.id === input.groupId) as Group
    const id = `e-${Date.now()}`
    world.environments.push({
      id,
      groupId: group.id,
      groupName: group.name,
      name: String(input.name).toLowerCase(),
      label: String(input.name),
      colorToken: String(input.colorToken ?? "env-local"),
    })
    return id
  },
  reorder_groups: () => null,
  create_group: (args) => {
    const g: Group = {
      id: `g-${Date.now()}`,
      name: String(a(args).name),
      displayOrder: world.groups.length,
      collapsed: false,
      connectionCount: 0,
    }
    world.groups.push(g)
    return g
  },
  rename_group: (args) =>
    Object.assign(world.groups.find((g) => g.id === a(args).id) as Group, { name: a(args).name }),
  delete_group: (args) => {
    world.groups = world.groups.filter((g) => g.id !== a(args).id)
    return null
  },
  set_group_collapsed: (args) => {
    const g = world.groups.find((x) => x.id === a(args).id)
    if (g) g.collapsed = Boolean(a(args).collapsed)
    return null
  },
  set_connection_collapsed: (args) => {
    connectionOf(a(args).connectionId).sidebarCollapsed = Boolean(a(args).collapsed)
    return null
  },

  // lock
  get_connection_lock_state: (args) =>
    isLocked(String(a(args).connectionId)) ? "locked" : "unlocked",
  unlock_connection: (args) => {
    const id = String(a(args).connectionId)
    world.unlocked.add(id)
    audit(id, "unlockConnection", { kind: "connection" })
    return "unlocked"
  },
  lock_connection: (args) => {
    const id = String(a(args).connectionId)
    world.unlocked.delete(id)
    audit(id, "lockConnection", { kind: "connection" })
    return "locked"
  },
} satisfies FakeHandlers
