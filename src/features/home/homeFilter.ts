import { hostPort } from "@/domain/bullmqEndpoint"
import { countOf, inLine } from "@/domain/stateCounts"
import type { GroupNode } from "@/features/connections"
import type { Connection, JobStateInfo, Queue } from "@/lib/tauri/bindings"

/**
 * The home page, as data (pure, unit-tested): the search across
 * groups, and the health summary of a connection.
 */

/** Words of the query must all appear in name / host:port / environment / group. */
export function matchesConnection(connection: Connection, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) return true
  const haystack = [
    connection.name,
    hostPort(connection) ?? "",
    connection.envLabel,
    connection.envName,
    connection.groupName,
  ]
    .join(" ")
    .toLowerCase()
  return words.every((w) => haystack.includes(w))
}

/** The tree, narrowed to what matches; empty environments and groups drop out. */
export function filterTree(groups: readonly GroupNode[], query: string): GroupNode[] {
  if (query.trim() === "") return [...groups]
  return groups
    .map((group) => ({
      ...group,
      envs: group.envs
        .map((env) => ({
          ...env,
          connections: env.connections.filter((c) => matchesConnection(c, query)),
        }))
        .filter((env) => env.connections.length > 0),
      connections: group.connections.filter((c) => matchesConnection(c, query)),
    }))
    .filter((group) => group.connections.length > 0)
}

export function countConnections(groups: readonly GroupNode[]): number {
  return groups.reduce((n, g) => n + g.connections.length, 0)
}

export interface ConnectionHealth {
  queues: number
  pausedQueues: number
  waiting: number
  active: number
  failed: number
  delayed: number
}

export function summarizeQueues(
  queues: readonly Queue[],
  states: ReadonlyArray<JobStateInfo>,
): ConnectionHealth {
  const health: ConnectionHealth = {
    queues: queues.length,
    pausedQueues: 0,
    waiting: 0,
    active: 0,
    failed: 0,
    delayed: 0,
  }
  for (const q of queues) {
    if (q.isPaused) health.pausedQueues += 1
    // Jobs parked in a paused queue, and in the backend's own waiting states, are waiting too.
    health.waiting += inLine(q.counts, states, true)
    health.active += countOf(q.counts, "active")
    health.failed += countOf(q.counts, "failed")
    health.delayed += countOf(q.counts, "delayed")
  }
  return health
}

/** 1234 → "1.2k", 1_250_000 → "1.3M"; cards have little room. */
export function compactCount(n: number): string {
  if (n < 1_000) return String(n)
  if (n < 1_000_000) return `${(n / 1_000).toFixed(n < 10_000 ? 1 : 0).replace(/\.0$/, "")}k`
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`
}
