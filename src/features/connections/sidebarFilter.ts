import type { Connection, Group, Queue } from "@/lib/tauri/bindings"

/**
 * Filter every connection's queues by name.
 * Pure: the sidebar passes the queue lists it already polls.
 */
export interface FilterMatchConnection {
  connection: Connection
  queues: Queue[]
}

export interface FilterMatchGroup {
  groupId: string
  groupName: string
  connections: FilterMatchConnection[]
}

export function filterQueues(
  query: string,
  groups: readonly Pick<Group, "id" | "name">[],
  connections: readonly Connection[],
  queuesOf: (connectionId: string) => readonly Queue[] | undefined,
): { groups: FilterMatchGroup[]; queueCount: number; connectionCount: number } {
  const needle = query.trim().toLowerCase()
  const out: FilterMatchGroup[] = []
  let queueCount = 0
  let connectionCount = 0
  for (const group of groups) {
    const matches: FilterMatchConnection[] = []
    for (const connection of connections.filter((c) => c.groupId === group.id)) {
      const queues = (queuesOf(connection.id) ?? [])
        .filter((q) => q.name.toLowerCase().includes(needle))
        .sort((a, b) => a.name.localeCompare(b.name))
      if (queues.length === 0) continue
      matches.push({ connection, queues })
      queueCount += queues.length
      connectionCount += 1
    }
    if (matches.length > 0)
      out.push({ groupId: group.id, groupName: group.name, connections: matches })
  }
  return { groups: out, queueCount, connectionCount }
}

/** `#1211` or bare digits: a job, not a queue, the palette looks it up. */
export function looksLikeJobId(query: string): boolean {
  return /^#?\d+$/.test(query.trim())
}

/** Split `text` around the first case-insensitive occurrence of `query`. */
export function highlightParts(text: string, query: string): [string, string, string] {
  const needle = query.trim().toLowerCase()
  const at = needle ? text.toLowerCase().indexOf(needle) : -1
  if (at < 0) return [text, "", ""]
  return [text.slice(0, at), text.slice(at, at + needle.length), text.slice(at + needle.length)]
}
