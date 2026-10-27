import type { Connection, Environment, Group } from "@/lib/tauri/bindings"

/**
 * Tree shape derived from the flat `Connection[]` returned by `list_connections`.
 *
 * The backend orders by group.display_order, env.display_order, conn.created_at —
 * we preserve that order strictly via `Map`-based grouping (Map iteration is
 * insertion order in ES2015+).
 *
 * The sidebar shows two levels, Group → Connection: the environment is a tag on
 * the connection, not a level. `envs` still holds a group's environments (the
 * tags it offers, in their order); `connections` is what is listed.
 */

export interface EnvNode {
  id: string
  name: string
  label: string
  colorToken: string
  connections: Connection[]
}

export interface GroupNode {
  id: string
  name: string
  /** Persisted sidebar state. */
  collapsed: boolean
  envs: EnvNode[]
  /** The group's connections: by environment order, then by name. */
  connections: Connection[]
}

const connectionsOf = (envs: EnvNode[]): Connection[] =>
  envs.flatMap((e) =>
    [...e.connections].sort((a, b) =>
      a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
    ),
  )

export function groupConnectionsByHierarchy(connections: Connection[]): GroupNode[] {
  const groupMap = new Map<string, GroupNode>()

  for (const c of connections) {
    let group = groupMap.get(c.groupId)
    if (!group) {
      group = { id: c.groupId, name: c.groupName, collapsed: false, envs: [], connections: [] }
      groupMap.set(c.groupId, group)
    }

    let env = group.envs.find((e) => e.id === c.environmentId)
    if (!env) {
      env = {
        id: c.environmentId,
        name: c.envName,
        label: c.envLabel,
        colorToken: c.envColor,
        connections: [],
      }
      group.envs.push(env)
    }

    env.connections.push(c)
  }

  return Array.from(groupMap.values()).map((g) => ({ ...g, connections: connectionsOf(g.envs) }))
}

/**
 * The sidebar tree. `list_groups` is the backbone (order, names,
 * collapse state, and crucially the groups that have no connection yet);
 * `list_connections` fills in environments and connections.
 *
 * A group present in `connections` but missing from `groups` (the two queries
 * resolve independently, so one can be a beat ahead of the other) is appended
 * rather than dropped — a connection must never vanish from the sidebar.
 */
export function buildSidebarTree(
  groups: Group[],
  connections: Connection[],
  environments: Environment[] = [],
): GroupNode[] {
  const fromConnections = new Map(groupConnectionsByHierarchy(connections).map((g) => [g.id, g]))

  const tree: GroupNode[] = groups.map((g) => {
    const envs = mergeEnvs(
      environments.filter((e) => e.groupId === g.id),
      fromConnections.get(g.id)?.envs ?? [],
    )
    return {
      id: g.id,
      name: g.name,
      collapsed: g.collapsed,
      envs,
      connections: connectionsOf(envs),
    }
  })

  const known = new Set(groups.map((g) => g.id))
  for (const node of fromConnections.values()) {
    if (!known.has(node.id)) tree.push(node)
  }
  return tree
}

/**
 * `list_environments` is the backbone of a group's environments
 * (order, label, color, and the ones left empty by a move, which stay as drop
 * targets). Same "never drop a connection" rule as for groups: an environment
 * only the connections query knows about is appended.
 */
function mergeEnvs(listed: Environment[], fromConnections: EnvNode[]): EnvNode[] {
  const byId = new Map(fromConnections.map((e) => [e.id, e]))
  const merged: EnvNode[] = listed.map((e) => ({
    id: e.id,
    name: e.name,
    label: e.label,
    colorToken: e.colorToken,
    connections: byId.get(e.id)?.connections ?? [],
  }))
  const known = new Set(listed.map((e) => e.id))
  for (const e of fromConnections) {
    if (!known.has(e.id)) merged.push(e)
  }
  return merged
}
