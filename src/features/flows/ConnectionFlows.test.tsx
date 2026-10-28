import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    listQueues: vi.fn(),
    listFlowRoots: vi.fn(),
    getFlow: vi.fn(),
    retryJob: vi.fn(),
    removeJob: vi.fn(),
    getConnectionLockState: vi.fn(),
  },
}))
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="#x">{children}</a>,
}))

import { commands, type FlowNode, type FlowRoot } from "@/lib/tauri/bindings"

import { ConnectionFlows, FlowInspector } from "./ConnectionFlows"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>
const ref = (name: string) => `bull:${name}`

const root = (id: string, blocked: boolean, timestamp: number): FlowRoot => ({
  jobRef: { queueId: ref("orders"), queueName: "orders", id },
  name: `order-${id}`,
  state: "waitingChildren",
  timestamp,
  childrenTotal: 3,
  childrenPending: 1,
  childrenCompleted: 1,
  childrenFailed: 1,
  blocked,
})

const node = (key: string, parentKey: string | null, extra: Partial<FlowNode> = {}): FlowNode => ({
  key,
  jobRef: {
    queueId: ref(key.split(":")[1] ?? "q"),
    queueName: key.split(":")[1] ?? "q",
    id: key.split(":")[2] ?? "1",
  },
  parentKey,
  name: key,
  state: "completed",
  timestamp: 0,
  durationMs: 1200,
  failedReason: null,
  missing: false,
  returnValue: null,
  blocksParent: false,
  ...extra,
})

function wrap(children: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={client}>{children}</QueryClientProvider>)
}

beforeEach(() => {
  mocked.listQueues.mockResolvedValue({
    status: "ok",
    data: [
      {
        queueId: ref("orders"),
        counts: {
          waiting: 0,
          active: 0,
          completed: 0,
          failed: 0,
          delayed: 0,
          prioritized: 0,
          paused: 0,
          waitingChildren: 2,
          stalled: 0,
        },
        isPaused: false,
      },
    ],
  })
  mocked.listFlowRoots.mockResolvedValue({
    status: "ok",
    data: {
      roots: [root("new", false, 2000), root("stuck", true, 1000)],
      scanned: 10,
      capped: false,
    },
  })
  mocked.getConnectionLockState.mockResolvedValue({ status: "ok", data: "unlocked" })
})
afterEach(() => vi.resetAllMocks())

describe("ConnectionFlows", () => {
  it("lists recent flows, a blocked one first, with how its children are doing", async () => {
    const onSelect = vi.fn()
    wrap(<ConnectionFlows connectionId="c1" selected={null} onSelect={onSelect} />)
    await waitFor(() => expect(screen.getAllByTestId("flow-row")).toHaveLength(2))
    const [first, second] = screen.getAllByTestId("flow-row")
    expect(first).toHaveAttribute("data-blocked", "true")
    expect(first).toHaveTextContent("order-stuck")
    expect(first).toHaveTextContent("Blocked")
    expect(first).toHaveTextContent("1/3 done · 1 failed")
    expect(second).toHaveTextContent("order-new")
    fireEvent.click(first as HTMLElement)
    expect(onSelect).toHaveBeenCalledWith("bull:orders/stuck")
  })

  it("the inspector: the tree, returned values, the failed child's error, and Retry child", async () => {
    mocked.getFlow.mockResolvedValue({
      status: "ok",
      data: {
        rootKey: "bull:orders:stuck",
        focusKey: "bull:orders:stuck",
        truncated: false,
        blocked: true,
        nodes: [
          node("bull:orders:stuck", null, { state: "waitingChildren" }),
          node("bull:emails:7", "bull:orders:stuck", { returnValue: '{"sent":true}' }),
          node("bull:emails:8", "bull:orders:stuck", {
            state: "failed",
            failedReason: "connect ETIMEDOUT\n  at x",
            blocksParent: true,
          }),
        ],
      },
    })
    mocked.retryJob.mockResolvedValue({
      status: "ok",
      data: { jobId: "8", newState: "waiting", action: "retry" },
    })
    wrap(<FlowInspector connectionId="c1" id="bull:orders/stuck" onClose={vi.fn()} />)
    await waitFor(() => expect(screen.getAllByTestId("flow-node")).toHaveLength(3))
    expect(screen.getByTestId("flow-node-returned")).toHaveTextContent('returned {"sent":true}')
    expect(screen.getByTestId("flow-tree")).toHaveTextContent("connect ETIMEDOUT")
    expect(screen.getByTestId("flow-blocked")).toHaveTextContent(
      "failed for good and its parent waits for it",
    )
    fireEvent.click(screen.getByTestId("flow-retry-child"))
    fireEvent.click(await screen.findByTestId("flow-child-confirm"))
    await waitFor(() =>
      expect(mocked.retryJob).toHaveBeenCalledWith({
        connectionId: "c1",
        queueId: ref("emails"),
        jobId: "8",
      }),
    )
  })
})
