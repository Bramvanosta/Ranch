import type { FlowNode } from "@/lib/tauri/bindings"

/**
 * top-to-bottom layout of a flow (pure, unit-tested).
 *
 * A flow is a tree, so no graph library is needed: leaves take the next free
 * column, a parent sits centred above its children, a level is a row. The
 * result is a set of boxes and the elbow connectors between them, in pixels,
 * that `FlowGraph` draws as absolutely-positioned buttons over one SVG.
 */
export const NODE_WIDTH = 236
export const NODE_HEIGHT = 64
export const GAP_X = 24
export const GAP_Y = 48

export interface PlacedNode {
  node: FlowNode
  x: number
  y: number
  depth: number
}

export interface PlacedEdge {
  from: string
  to: string
  /** SVG path: down from the parent, across, down into the child. */
  path: string
}

export interface FlowLayout {
  nodes: PlacedNode[]
  edges: PlacedEdge[]
  width: number
  height: number
}

export function layoutFlow(nodes: readonly FlowNode[], rootKey: string): FlowLayout {
  const byKey = new Map(nodes.map((n) => [n.key, n]))
  const children = new Map<string, FlowNode[]>()
  for (const n of nodes) {
    if (n.parentKey && byKey.has(n.parentKey)) {
      const list = children.get(n.parentKey) ?? []
      list.push(n)
      children.set(n.parentKey, list)
    }
  }
  const root = byKey.get(rootKey) ?? nodes[0]
  if (!root) return { nodes: [], edges: [], width: 0, height: 0 }

  const placed = new Map<string, PlacedNode>()
  let nextColumn = 0
  let maxDepth = 0

  // Post-order: children first, then the parent centred over them.
  const place = (node: FlowNode, depth: number): number => {
    maxDepth = Math.max(maxDepth, depth)
    const kids = children.get(node.key) ?? []
    let column: number
    if (kids.length === 0) {
      column = nextColumn
      nextColumn += 1
    } else {
      const columns = kids.map((k) => place(k, depth + 1))
      column = (columns[0] + columns[columns.length - 1]) / 2
    }
    placed.set(node.key, {
      node,
      depth,
      x: column * (NODE_WIDTH + GAP_X),
      y: depth * (NODE_HEIGHT + GAP_Y),
    })
    return column
  }
  place(root, 0)

  const edges: PlacedEdge[] = []
  for (const [parentKey, kids] of children) {
    const parent = placed.get(parentKey)
    if (!parent) continue
    for (const kid of kids) {
      const child = placed.get(kid.key)
      if (!child) continue
      const x1 = parent.x + NODE_WIDTH / 2
      const y1 = parent.y + NODE_HEIGHT
      const x2 = child.x + NODE_WIDTH / 2
      const y2 = child.y
      const mid = y1 + GAP_Y / 2
      edges.push({
        from: parentKey,
        to: kid.key,
        path: `M ${x1} ${y1} V ${mid} H ${x2} V ${y2}`,
      })
    }
  }

  return {
    // Root first, then row by row, left to right: a stable reading / tab order.
    nodes: [...placed.values()].sort((a, b) => a.depth - b.depth || a.x - b.x),
    edges,
    width: Math.max(1, nextColumn) * (NODE_WIDTH + GAP_X) - GAP_X,
    height: (maxDepth + 1) * (NODE_HEIGHT + GAP_Y) - GAP_Y,
  }
}

/** Children of a flow by what became of them — for the header line. */
export function summarizeFlow(nodes: readonly FlowNode[]) {
  const count = (state: FlowNode["state"]) => nodes.filter((n) => n.state === state).length
  return {
    total: nodes.length,
    completed: count("completed"),
    failed: count("failed"),
    active: count("active"),
    waitingChildren: count("waitingChildren"),
  }
}
