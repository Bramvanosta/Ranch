/**
 * The queues table.
 */
import { fireEvent, screen, waitFor, within } from "@testing-library/react"
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

import { commands } from "@/lib/tauri/bindings"
import { aQueue, aQueueId } from "@/test/factories"
import { renderWithProviders } from "@/test/renderWithProviders"
import { err, ok } from "@/test/results"

import { QueuesTable } from "./QueuesTable"

const listQueues = vi.mocked(commands.listQueues)
const countQueueWorkers = vi.mocked(commands.countQueueWorkers)
const getThroughput = vi.mocked(commands.getThroughput)

afterEach(() => vi.resetAllMocks())

function renderTable(connectionId = "conn-x") {
  return renderWithProviders(<QueuesTable connectionId={connectionId} />)
}

const counters = (c: Record<string, number>) => ({ ...aQueue().counts, ...c })

function seed() {
  listQueues.mockResolvedValue(
    ok([
      aQueue("emails", { counts: counters({ waiting: 30, active: 2, failed: 12 }) }),
      aQueue("images", { counts: counters({ waiting: 5 }), isPaused: true }),
      aQueue("reports", { counts: counters({ waiting: 4 }) }),
      aQueue("orders", { counts: counters({}) }),
    ]),
  )
  countQueueWorkers.mockImplementation(async ({ queueIds }) =>
    ok({
      supported: true,
      counts: queueIds.map((r) => ({ queueId: r, workers: r === "bull:emails" ? 3 : 0 })),
    }),
  )
  getThroughput.mockResolvedValue(
    ok({
      range: "1h",
      bucketMs: 1,
      bucketStartsMs: [0, 1],
      queues: [
        {
          queueId: aQueueId("emails"),
          queueName: "emails",
          metricsOff: false,
          completed: [60, 30],
          failed: [5, 5],
        },
        {
          queueId: aQueueId("images"),
          queueName: "images",
          metricsOff: false,
          completed: [0, 0],
          failed: [0, 0],
        },
        {
          queueId: aQueueId("reports"),
          queueName: "reports",
          metricsOff: true,
          completed: [],
          failed: [],
        },
        {
          queueId: aQueueId("orders"),
          queueName: "orders",
          metricsOff: false,
          completed: [1, 0],
          failed: [0, 0],
        },
      ],
      completed: [61, 30],
      failed: [5, 5],
      metricsOffQueues: 1,
    }),
  )
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
      "/connections/conn-x/queues/bull%3Aemails",
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
    await waitFor(() => expect(within(reports).getAllByText("-").length).toBeGreaterThan(0))
    expect((within(reports).getAllByText("-")[0] as HTMLElement).className).toContain("--text2")
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
    const { client } = renderTable()
    expect(await screen.findByText("emails")).toBeInTheDocument()

    listQueues.mockResolvedValue(err("redis", "timed out"))
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
    listQueues.mockResolvedValue(ok([]))
    renderTable()
    expect(await screen.findByText("No BullMQ queues found.")).toBeInTheDocument()
  })
})
