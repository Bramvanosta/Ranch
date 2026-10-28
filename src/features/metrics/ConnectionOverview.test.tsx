/**
 * ConnectionOverview integration tests.
 */
import { QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { testQueryClient } from "@/test/renderWithProviders"

vi.mock("recharts", () => {
  const Passthrough = ({ children }: { children?: React.ReactNode }) => (
    <div data-stub="chart">{children}</div>
  )
  const Leaf = () => null
  return {
    ResponsiveContainer: Passthrough,
    BarChart: Passthrough,
    LineChart: Passthrough,
    CartesianGrid: Leaf,
    XAxis: Leaf,
    YAxis: Leaf,
    Bar: Leaf,
    Line: Leaf,
    Tooltip: Leaf,
    Legend: Leaf,
  }
})

// The recent-jobs card has its own tests (features/activity); here it would
// only add a second command to mock.
vi.mock("@/features/activity/RecentJobsCard", () => ({
  RecentJobsCard: () => <div data-testid="recent-jobs-stub" />,
}))
// The live events panel has its own tests (features/overview).
vi.mock("@/features/overview/LiveEvents", () => ({
  LiveEvents: () => <div data-testid="live-events-stub" />,
}))

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    getConnectionMetrics: vi.fn(),
    listQueues: vi.fn(),
    getThroughput: vi.fn(),
    listFailureCauses: vi.fn(),
    countQueueWorkers: vi.fn(),
  },
}))
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, ...rest }: { children: React.ReactNode } & Record<string, unknown>) => (
    <a {...rest}>{children}</a>
  ),
}))

import { commands } from "@/lib/tauri/bindings"

import { ConnectionOverview } from "./ConnectionOverview"
import { emptyMetrics } from "./testMetrics"

const mocked = commands as unknown as {
  getConnectionMetrics: ReturnType<typeof vi.fn>
  listQueues: ReturnType<typeof vi.fn>
  getThroughput: ReturnType<typeof vi.fn>
  listFailureCauses: ReturnType<typeof vi.fn>
  countQueueWorkers: ReturnType<typeof vi.fn>
}

const metrics = emptyMetrics

function renderOverview() {
  const client = testQueryClient()
  return render(
    <QueryClientProvider client={client}>
      <ConnectionOverview connectionId="conn-1" range="24h" onRangeChange={vi.fn()} />
    </QueryClientProvider>,
  )
}

afterEach(() => {
  vi.resetAllMocks()
})

// Default: no queues. Individual tests override.
function setQueues(
  queues: Array<{ name: string; failed: number; waiting?: number; paused?: boolean }>,
) {
  mocked.listQueues.mockResolvedValue({
    status: "ok",
    data: queues.map((q) => ({
      id: `bull:${q.name}`,
      name: q.name,
      counts: {
        waiting: q.waiting ?? 0,
        active: 0,
        completed: 0,
        failed: q.failed,
        delayed: 0,
        prioritized: 0,
        paused: 0,
        waitingChildren: 0,
        stalled: 0,
      },
      isPaused: q.paused ?? false,
    })),
  })
}

const ref = (name: string) => `bull:${name}`

describe("ConnectionOverview", () => {
  it("renders the loading state initially", () => {
    mocked.getConnectionMetrics.mockReturnValue(new Promise(() => {}))
    setQueues([])
    renderOverview()
    expect(screen.getByTestId("connection-overview-loading")).toBeInTheDocument()
  })

  it("renders the dashboard on success", async () => {
    mocked.getConnectionMetrics.mockResolvedValue({
      status: "ok",
      data: metrics({ completedTotal: 500, throughputPerMinute: 20.8 }),
    })
    setQueues([])
    renderOverview()
    await waitFor(() => {
      expect(screen.getByTestId("overview-dashboard")).toBeInTheDocument()
    })
  })

  it("renders the error state on failure", async () => {
    mocked.getConnectionMetrics.mockResolvedValue({
      status: "error",
      error: { kind: "schemaNotRecognized", message: "kaboom" },
    })
    setQueues([])
    renderOverview()
    await waitFor(() => {
      expect(screen.getByTestId("connection-overview-error")).toBeInTheDocument()
    })
    expect(screen.getByTestId("connection-overview-error")).toHaveTextContent("kaboom")
  })

  it("five tiles: the counters, failures of the last hour, completed per hour", async () => {
    mocked.getConnectionMetrics.mockResolvedValue({ status: "ok", data: metrics() })
    setQueues([
      { name: "emails", failed: 12, waiting: 30 },
      { name: "images", failed: 0, waiting: 5 },
    ])
    mocked.getThroughput.mockImplementation(async ({ range }: { range: string }) => ({
      status: "ok",
      data: {
        range,
        bucketMs: 1,
        bucketStartsMs: [0, 1],
        queues: [
          {
            queueId: ref("emails"),
            queueName: "emails",
            metricsOff: false,
            completed: [10, 20],
            failed: [1, 2],
          },
          {
            queueId: ref("images"),
            queueName: "images",
            metricsOff: true,
            completed: [],
            failed: [],
          },
        ],
        completed: [10, 20],
        failed: [1, 2],
        metricsOffQueues: 1,
      },
    }))
    renderOverview()
    await waitFor(() => expect(screen.getByTestId("overview-tile-waiting")).toHaveTextContent("35"))
    expect(screen.getByTestId("overview-tile-failed")).toHaveTextContent("12")
    await waitFor(() =>
      expect(screen.getByTestId("overview-tile-failed")).toHaveTextContent("+3 in the last hour"),
    )
    expect(screen.getByTestId("overview-tile-completed")).toHaveTextContent("30")
    expect(screen.getByTestId("overview-tile-completed")).toHaveTextContent(
      "1 of 2 queues record no metrics",
    )
  })

  it("needs attention: causes, paused queues and queues without a worker, each with Open", async () => {
    mocked.getConnectionMetrics.mockResolvedValue({ status: "ok", data: metrics() })
    setQueues([
      { name: "emails", failed: 3 },
      { name: "reports", failed: 0, waiting: 5, paused: true },
      { name: "images", failed: 0, waiting: 8 },
    ])
    mocked.countQueueWorkers.mockResolvedValue({
      status: "ok",
      data: {
        supported: true,
        counts: [
          { queueId: ref("emails"), workers: 2 },
          { queueId: ref("reports"), workers: 0 },
          { queueId: ref("images"), workers: 0 },
        ],
      },
    })
    mocked.listFailureCauses.mockResolvedValue({
      status: "ok",
      data: {
        causes: [
          {
            signature: "connect ETIMEDOUT <ip>",
            sample: "connect ETIMEDOUT 10.0.4.12:5432",
            count: 3,
            firstAt: Date.now() - 60_000,
            lastAt: Date.now() - 1_000,
            jobNames: ["send"],
            queueNames: ["emails"],
            trend: [],
            jobs: [],
          },
        ],
        scanned: 3,
        scanCapped: false,
        totalFailed: 3,
        unreadableQueues: [],
      },
    })
    renderOverview()
    expect(await screen.findByTestId("attention-cause")).toHaveTextContent(
      "3 connect ETIMEDOUT <ip>",
    )
    expect(screen.getByTestId("attention-paused")).toHaveTextContent("reports is paused")
    // A paused queue is not also "without a worker".
    await waitFor(() =>
      expect(screen.getByTestId("attention-no-worker")).toHaveTextContent("images has no worker"),
    )
    expect(screen.getAllByTestId("attention-no-worker")).toHaveLength(1)
    for (const row of screen.getByTestId("needs-attention").querySelectorAll("li")) {
      expect(row).toHaveTextContent("Open")
    }
  })

  it("busiest queues: most active first, metrics off last", async () => {
    mocked.getConnectionMetrics.mockResolvedValue({ status: "ok", data: metrics() })
    setQueues([])
    mocked.getThroughput.mockResolvedValue({
      status: "ok",
      data: {
        range: "6h",
        bucketMs: 1,
        bucketStartsMs: [0],
        queues: [
          {
            queueId: ref("quiet"),
            queueName: "quiet",
            metricsOff: false,
            completed: [1],
            failed: [0],
          },
          { queueId: ref("off"), queueName: "off", metricsOff: true, completed: [], failed: [] },
          {
            queueId: ref("busy"),
            queueName: "busy",
            metricsOff: false,
            completed: [90],
            failed: [4],
          },
        ],
        completed: [91],
        failed: [4],
        metricsOffQueues: 1,
      },
    })
    renderOverview()
    await waitFor(() => expect(screen.getAllByTestId("busiest-queue")).toHaveLength(3))
    const rows = screen.getAllByTestId("busiest-queue")
    expect(rows[0]).toHaveTextContent("busy")
    expect(rows[0]).toHaveTextContent("90 done · 4 failed")
    expect(rows[2]).toHaveTextContent("off")
    expect(rows[2]).toHaveTextContent("metrics off")
  })

  it("says how many queues are estimated at connection level", async () => {
    mocked.listQueues.mockResolvedValue({ status: "ok", data: [] })
    mocked.getConnectionMetrics.mockResolvedValue({
      status: "ok",
      data: metrics({ nativeQueues: 1, totalQueues: 3 }),
    })
    renderOverview()
    expect(await screen.findByTestId("metrics-estimate-notice")).toHaveTextContent(
      "2 of 3 queues do not record BullMQ metrics",
    )
  })
})
