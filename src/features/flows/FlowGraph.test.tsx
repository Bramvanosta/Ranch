/**
 * The flow graph, the queue's Flows tab and the job's Flow tab.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, within } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

const navigate = vi.fn()
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: { getFlow: vi.fn(), listFlowRoots: vi.fn() },
}))

import { commands, type FlowNode, type FlowTree } from "@/lib/tauri/bindings"

import { FlowGraph } from "./FlowGraph"
import { FlowsView } from "./FlowsView"
import { JobFlowTab } from "./JobFlowTab"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>

afterEach(() => {
  vi.resetAllMocks()
})

function node(
  queue: string,
  id: string,
  parent: string | null,
  over: Partial<FlowNode> = {},
): FlowNode {
  return {
    key: `bull:${queue}:${id}`,
    jobRef: { queue: { prefix: "bull", name: queue }, id },
    parentKey: parent,
    name: `job-${id}`,
    state: "completed",
    timestamp: 1,
    durationMs: 1200,
    failedReason: null,
    missing: false,
    ...over,
  }
}

const tree: FlowTree = {
  nodes: [
    node("orders", "1", null, { name: "assemble", state: "waitingChildren", durationMs: null }),
    node("parts", "2", "bull:orders:1", { name: "fetch-ok" }),
    node("parts", "3", "bull:orders:1", {
      name: "fetch-bad",
      state: "failed",
      failedReason: "upstream 503",
    }),
    node("parts", "4", "bull:orders:1", { name: "gone", missing: true, state: "unknown" }),
  ],
  rootKey: "bull:orders:1",
  focusKey: "bull:parts:3",
  truncated: false,
}

function wrap(ui: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>)
}

describe("FlowGraph", () => {
  it("draws every job with state, queue and duration, the current one highlighted", () => {
    wrap(<FlowGraph connectionId="c1" tree={tree} />)
    const nodes = screen.getAllByTestId("flow-node")
    expect(nodes).toHaveLength(4)
    // Reading order: root first.
    expect(nodes[0]).toHaveTextContent("assemble")
    expect(nodes[0]).toHaveTextContent("waiting-children")
    expect(nodes[0]).toHaveTextContent("orders #1")

    const bad = screen.getByRole("button", { name: /fetch-bad, failed, queue parts, job 3/ })
    expect(bad).toHaveAttribute("aria-current", "true")
    expect(bad).toHaveAttribute("title", "upstream 503")
    expect(within(bad).getByText("1.20 s")).toBeInTheDocument()
    expect(screen.getByTestId("flow-graph-summary")).toHaveTextContent(
      "4 jobs · 1 completed · 1 failed · 1 waiting for children",
    )
  })

  it("goes to the clicked job — in another queue of the connection too", () => {
    wrap(<FlowGraph connectionId="c1" tree={tree} />)
    fireEvent.click(screen.getByRole("button", { name: /fetch-ok/ }))
    expect(navigate).toHaveBeenCalledWith({
      to: "/connections/$connectionId/queues/$queueName/$jobId",
      params: { connectionId: "c1", queueName: "parts", jobId: "2" },
      search: { tab: "flow" },
    })
  })

  it("cannot open a job that no longer exists, and says when the flow is cut", () => {
    wrap(<FlowGraph connectionId="c1" tree={{ ...tree, truncated: true }} />)
    expect(screen.getByRole("button", { name: /gone/ })).toBeDisabled()
    expect(screen.getByTestId("flow-graph-summary")).toHaveTextContent(
      "only the first 200 jobs are shown",
    )
  })
})

describe("JobFlowTab", () => {
  it("shows the graph of the job's flow", async () => {
    mocked.getFlow.mockResolvedValue({ status: "ok", data: tree })
    wrap(<JobFlowTab connectionId="c1" queueRef={{ prefix: "bull", name: "parts" }} jobId="3" />)
    expect(await screen.findByTestId("flow-graph")).toBeInTheDocument()
    expect(mocked.getFlow).toHaveBeenCalledWith({
      connectionId: "c1",
      queueRef: { prefix: "bull", name: "parts" },
      jobId: "3",
    })
  })

  it("says so when the job is in no flow", async () => {
    mocked.getFlow.mockResolvedValue({
      status: "ok",
      data: { ...tree, nodes: [tree.nodes[0]], focusKey: "bull:orders:1" },
    })
    wrap(<JobFlowTab connectionId="c1" queueRef={{ prefix: "bull", name: "orders" }} jobId="1" />)
    expect(await screen.findByTestId("job-flow-none")).toBeInTheDocument()
  })
})

describe("FlowsView", () => {
  const queueRef = { prefix: "bull", name: "orders" }
  const root = {
    jobRef: { queue: queueRef, id: "1" },
    name: "assemble",
    state: "waitingChildren" as const,
    timestamp: Date.now() - 120_000,
    childrenTotal: 3,
    childrenPending: 0,
    childrenCompleted: 2,
    childrenFailed: 1,
  }

  it("lists the flow roots with what became of their children, expandable to the graph", async () => {
    mocked.listFlowRoots.mockResolvedValue({
      status: "ok",
      data: { roots: [root], scanned: 40, capped: false },
    })
    mocked.getFlow.mockResolvedValue({ status: "ok", data: tree })
    wrap(<FlowsView connectionId="c1" queueRef={queueRef} />)

    const row = await screen.findByTestId("flow-root")
    expect(row).toHaveTextContent("assemble")
    expect(row).toHaveTextContent("2/3 done · 1 failed")
    expect(row).toHaveTextContent("waiting-children")
    expect(row).toHaveAttribute("aria-expanded", "false")
    expect(screen.queryByTestId("flow-graph")).not.toBeInTheDocument()

    fireEvent.click(row)
    expect(await screen.findByTestId("flow-graph")).toBeInTheDocument()
    expect(row).toHaveAttribute("aria-expanded", "true")
    // No job is "the current one" here.
    expect(screen.queryByRole("button", { current: true })).not.toBeInTheDocument()
  })

  it("explains where flows come from when there is none, and the bound when capped", async () => {
    mocked.listFlowRoots.mockResolvedValue({
      status: "ok",
      data: { roots: [], scanned: 1000, capped: true },
    })
    wrap(<FlowsView connectionId="c1" queueRef={queueRef} />)
    const empty = await screen.findByTestId("flows-empty")
    expect(empty).toHaveTextContent("FlowProducer")
    expect(empty).toHaveTextContent("older flows are not listed")
  })
})
