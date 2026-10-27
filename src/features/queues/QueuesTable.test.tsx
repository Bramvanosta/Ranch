/**
 * The queues table.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import type { ComponentProps, ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listQueues: vi.fn(), countQueueWorkers: vi.fn(), getThroughput: vi.fn() },
}))
// jsdom has no layout: the virtualizer would show no row. Show them all.
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count, estimateSize }: { count: number; estimateSize: () => number }) => ({
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        index,
        key: index,
        start: index * estimateSize(),
      })),
    getTotalSize: () => count * estimateSize(),
    scrollToIndex: () => {},
    measureElement: () => {},
  }),
}))
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  Link: ({
    to,
    params,
    children,
    ...rest
  }: {
    to: string
    params?: Record<string, string>
    children: ReactNode
  } & ComponentProps<"a">) => {
    const href = Object.entries(params ?? {}).reduce(
      (acc, [k, v]) => acc.replace(`$${k}`, encodeURIComponent(v)),
      to,
    )
    return (
      <a href={href} {...rest}>
        {children}
      </a>
    )
  },
}))

import { commands, type Queue } from "@/lib/tauri/bindings"

import { QueuesTable } from "./QueuesTable"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>

function makeQueue(name: string, overrides: Partial<Queue> = {}): Queue {
  return {
    queueRef: { prefix: "bull", name },
    counters: {
      waiting: 0,
      active: 0,
      completed: 0,
      failed: 0,
      delayed: 0,
      prioritized: 0,
      paused: 0,
      waitingChildren: 0,
      stalled: 0,
    },
    isPaused: false,
    ...overrides,
  } as unknown as Queue
}

afterEach(() => vi.resetAllMocks())

function renderTable(connectionId = "conn-x") {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <QueuesTable connectionId={connectionId} />
    </QueryClientProvider>,
  )
}

const counters = (c: Partial<Queue["counters"]>) =>
  ({
    waiting: 0,
    active: 0,
    completed: 0,
    failed: 0,
    delayed: 0,
    prioritized: 0,
    paused: 0,
    waitingChildren: 0,
    stalled: 0,
    ...c,
  }) as Queue["counters"]

function seed() {
  mocked.listQueues.mockResolvedValue({
    status: "ok",
    data: [
      makeQueue("emails", { counters: counters({ waiting: 30, active: 2, failed: 12 }) }),
      makeQueue("images", { counters: counters({ waiting: 5 }), isPaused: true }),
      makeQueue("reports", { counters: counters({ waiting: 4 }) }),
      makeQueue("orders", { counters: counters({}) }),
    ],
  })
  mocked.countQueueWorkers.mockImplementation(
    async ({ queueRefs }: { queueRefs: Array<{ name: string }> }) => ({
      status: "ok",
      data: {
        supported: true,
        counts: queueRefs.map((r) => ({ queueRef: r, workers: r.name === "emails" ? 3 : 0 })),
      },
    }),
  )
  const ref = (name: string) => ({ prefix: "bull", name })
  mocked.getThroughput.mockResolvedValue({
    status: "ok",
    data: {
      range: "1h",
      bucketMs: 1,
      bucketStartsMs: [0, 1],
      queues: [
        { queueRef: ref("emails"), metricsOff: false, completed: [60, 30], failed: [5, 5] },
        { queueRef: ref("images"), metricsOff: false, completed: [0, 0], failed: [0, 0] },
        { queueRef: ref("reports"), metricsOff: true, completed: [], failed: [] },
        { queueRef: ref("orders"), metricsOff: false, completed: [1, 0], failed: [0, 0] },
      ],
      completed: [61, 30],
      failed: [5, 5],
      metricsOffQueues: 1,
    },
  })
}

describe("QueuesTable", () => {
  it("one row per queue with its counters, done per hour and fail rate", async () => {
    seed()
    renderTable()
    const emails = await screen.findByTestId("queue-row-emails")
    await waitFor(() => expect(emails).toHaveTextContent("90"))
    expect(emails).toHaveTextContent("30")
    expect(emails).toHaveTextContent("12")
    expect(within(emails).getByRole("link", { name: "emails" })).toHaveAttribute(
      "href",
      "/connections/conn-x/queues/emails",
    )
    expect(
      screen.getAllByRole("row").filter((r) => r.dataset.testid?.startsWith("queue-row")),
    ).toHaveLength(4)
  })

  it("says Paused and No worker in their colours", async () => {
    seed()
    renderTable()
    expect(await screen.findByTestId("queue-status-images")).toHaveTextContent("Paused")
    await waitFor(() =>
      expect(screen.getByTestId("queue-status-reports")).toHaveTextContent("No worker"),
    )
    expect(screen.getByTestId("queue-status-reports").className).toContain("state-failed-text")
    expect(screen.getByTestId("queue-status-images").className).toContain("state-delayed-text")
    expect(screen.getByTestId("queue-status-emails")).toHaveTextContent("Working")
    expect(screen.getByTestId("queue-status-orders")).toHaveTextContent("Idle")
  })

  it("zeros are dim, unknown is a readable dash", async () => {
    seed()
    renderTable()
    const orders = await screen.findByTestId("queue-row-orders")
    const zero = within(orders).getAllByText("0")[0] as HTMLElement
    expect(zero.className).toContain("--dim")
    const reports = screen.getByTestId("queue-row-reports")
    await waitFor(() => expect(within(reports).getAllByText("—").length).toBeGreaterThan(0))
    expect((within(reports).getAllByText("—")[0] as HTMLElement).className).toContain("--text2")
  })

  it("sorts on a numeric column, then the other way", async () => {
    seed()
    renderTable()
    await screen.findByTestId("queue-row-emails")
    const order = () =>
      screen
        .getAllByRole("row")
        .map((r) => r.dataset.testid)
        .filter((id): id is string => id?.startsWith("queue-row") ?? false)
    fireEvent.click(screen.getByTestId("queues-sort-waiting"))
    expect(order()).toEqual([
      "queue-row-emails",
      "queue-row-images",
      "queue-row-reports",
      "queue-row-orders",
    ])
    fireEvent.click(screen.getByTestId("queues-sort-waiting"))
    expect(order()[0]).toBe("queue-row-orders")
  })

  it("keeps the queues on screen when a refresh fails, and says so", async () => {
    seed()
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={client}>
        <QueuesTable connectionId="conn-x" />
      </QueryClientProvider>,
    )
    expect(await screen.findByText("emails")).toBeInTheDocument()

    mocked.listQueues.mockResolvedValue({
      status: "error",
      error: { kind: "redis", message: "timed out", docUrl: null },
    })
    await client.refetchQueries({ queryKey: ["queues", "conn-x"] }).catch(() => undefined)

    await waitFor(() => expect(screen.getByTestId("stale-notice")).toBeInTheDocument(), {
      timeout: 4_000,
    })
    expect(screen.getByTestId("stale-notice")).toHaveTextContent("Could not refresh the queues")
    expect(screen.getByTestId("stale-notice")).toHaveTextContent("timed out")
    expect(screen.getByText("emails")).toBeInTheDocument()
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
  })

  it("shows the empty state when no queues are found", async () => {
    mocked.listQueues.mockResolvedValue({ status: "ok", data: [] })
    renderTable()
    expect(await screen.findByText("No BullMQ queues found.")).toBeInTheDocument()
  })
})
