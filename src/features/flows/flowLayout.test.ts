import { describe, expect, it } from "vitest"

import type { FlowNode } from "@/lib/tauri/bindings"

import { GAP_X, GAP_Y, layoutFlow, NODE_HEIGHT, NODE_WIDTH, summarizeFlow } from "./flowLayout"

function node(id: string, parent: string | null, state: FlowNode["state"] = "completed"): FlowNode {
  return {
    key: `bull:q:${id}`,
    jobRef: { queueId: "bull:q", queueName: "q", id },
    parentKey: parent ? `bull:q:${parent}` : null,
    name: `job-${id}`,
    state,
    timestamp: 1,
    durationMs: null,
    failedReason: null,
    missing: false,
  }
}

const COL = NODE_WIDTH + GAP_X
const ROW = NODE_HEIGHT + GAP_Y

describe("layoutFlow", () => {
  it("puts a parent centred above its children, one row per level", () => {
    //        root
    //   a     b     c
    //        b1 b2
    const layout = layoutFlow(
      [
        node("root", null),
        node("a", "root"),
        node("b", "root"),
        node("c", "root"),
        node("b1", "b"),
        node("b2", "b"),
      ],
      "bull:q:root",
    )
    const at = (id: string) => {
      const p = layout.nodes.find((n) => n.node.jobRef.id === id)
      return p ? [p.x, p.y] : null
    }
    expect(at("a")).toEqual([0, ROW])
    expect(at("b1")).toEqual([COL, 2 * ROW])
    expect(at("b2")).toEqual([2 * COL, 2 * ROW])
    expect(at("b")).toEqual([1.5 * COL, ROW])
    expect(at("c")).toEqual([3 * COL, ROW])
    expect(at("root")).toEqual([1.5 * COL, 0])
    expect(layout.width).toBe(4 * COL - GAP_X)
    expect(layout.height).toBe(3 * ROW - GAP_Y)
    // Reading order: root, then rows left to right.
    expect(layout.nodes.map((n) => n.node.jobRef.id)).toEqual(["root", "a", "b", "c", "b1", "b2"])
  })

  it("draws one elbow connector per parent → child", () => {
    const layout = layoutFlow([node("root", null), node("a", "root")], "bull:q:root")
    expect(layout.edges).toEqual([
      {
        from: "bull:q:root",
        to: "bull:q:a",
        path: `M ${NODE_WIDTH / 2} ${NODE_HEIGHT} V ${NODE_HEIGHT + GAP_Y / 2} H ${NODE_WIDTH / 2} V ${ROW}`,
      },
    ])
  })

  it("survives a single job, an unknown root and an orphan", () => {
    expect(layoutFlow([node("solo", null)], "bull:q:solo")).toMatchObject({
      width: NODE_WIDTH,
      height: NODE_HEIGHT,
      edges: [],
    })
    expect(layoutFlow([], "x")).toEqual({ nodes: [], edges: [], width: 0, height: 0 })
    // A node whose parent is not in the tree (truncated flow) is not drawn.
    const layout = layoutFlow([node("root", null), node("lost", "gone")], "bull:q:root")
    expect(layout.nodes.map((n) => n.node.jobRef.id)).toEqual(["root"])
  })
})

describe("summarizeFlow", () => {
  it("counts jobs by what became of them", () => {
    expect(
      summarizeFlow([
        node("root", null, "waitingChildren"),
        node("a", "root", "completed"),
        node("b", "root", "failed"),
        node("c", "root", "active"),
      ]),
    ).toEqual({ total: 4, completed: 1, failed: 1, active: 1, waitingChildren: 1 })
  })
})
