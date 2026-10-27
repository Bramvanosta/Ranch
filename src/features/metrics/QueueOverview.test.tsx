/**
 * QueueOverview integration tests.
 *
 * Mocks the Tauri binding so we can drive the component through its
 * loading → ok → error states without touching Redis.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("recharts", () => {
  // Recharts uses ResizeObserver internally + measures via canvas — render
  // a deterministic stub so the assertions work in jsdom.
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

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children?: React.ReactNode }) => <a href="#stub">{children}</a>,
}))

// The recent-jobs card has its own tests (features/activity); here it would
// only add a second command to mock.
vi.mock("@/features/activity/RecentJobsCard", () => ({
  RecentJobsCard: () => <div data-testid="recent-jobs-stub" />,
}))

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { getQueueMetrics: vi.fn() },
}))

import { commands } from "@/lib/tauri/bindings"

import { QueueOverview } from "./QueueOverview"
import { emptyMetrics } from "./testMetrics"

const mocked = commands as unknown as { getQueueMetrics: ReturnType<typeof vi.fn> }

const metrics = emptyMetrics

const onRangeChange = vi.fn()

function renderOverview() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <QueueOverview
        connectionId="conn-1"
        queueRef={{ prefix: "bull", name: "emails" }}
        range="24h"
        onRangeChange={onRangeChange}
      />
    </QueryClientProvider>,
  )
}

afterEach(() => {
  vi.resetAllMocks()
})

describe("QueueOverview", () => {
  it("renders the loading state before data arrives", () => {
    mocked.getQueueMetrics.mockReturnValue(new Promise(() => {})) // never resolves
    renderOverview()
    expect(screen.getByTestId("queue-overview-loading")).toBeInTheDocument()
  })

  it("renders the dashboard with the 4 KPI cards on success", async () => {
    mocked.getQueueMetrics.mockResolvedValue({
      status: "ok",
      data: metrics({
        completedTotal: 240,
        failedTotal: 12,
        failureRate: 0.05,
        throughputPerMinute: 10,
        avgProcessingTimeMs: 1500,
        avgQueueDelayMs: 200,
      }),
    })
    renderOverview()
    await waitFor(() => {
      expect(screen.getByTestId("overview-dashboard")).toBeInTheDocument()
    })
    expect(screen.getByText("Throughput")).toBeInTheDocument()
    expect(screen.getByText("Failure rate")).toBeInTheDocument()
    expect(screen.getByText("Processing time")).toBeInTheDocument()
    expect(screen.getByText("Queue delay")).toBeInTheDocument()
  })

  it("renders the error state on failure", async () => {
    mocked.getQueueMetrics.mockResolvedValue({
      status: "error",
      error: { kind: "schemaNotRecognized", message: "boom" },
    })
    renderOverview()
    await waitFor(() => {
      expect(screen.getByTestId("queue-overview-error")).toBeInTheDocument()
    })
    expect(screen.getByTestId("queue-overview-error")).toHaveTextContent("boom")
  })

  it("colors the failure rate red when ≥10%", async () => {
    mocked.getQueueMetrics.mockResolvedValue({
      status: "ok",
      data: metrics({ failureRate: 0.2, failedTotal: 50, completedTotal: 200 }),
    })
    renderOverview()
    await waitFor(() => {
      expect(screen.getByTestId("overview-dashboard")).toBeInTheDocument()
    })
    // The Failure rate card's value span should carry the red class.
    const failureCard = screen.getByTestId("metric-card-failure-rate")
    const value = failureCard.querySelector('[data-testid="metric-card-value"]')
    expect(value?.className).toMatch(/text-\(--state-failed-text\)/)
  })

  // ── Time window ───────────────────────────────────────────────────────

  it("asks the backend for the selected window and reports a change of window", async () => {
    mocked.getQueueMetrics.mockResolvedValue({ status: "ok", data: metrics() })
    renderOverview()
    await screen.findByTestId("overview-dashboard")
    expect(mocked.getQueueMetrics).toHaveBeenCalledWith(
      expect.objectContaining({ range: "24h", nowMs: null }),
    )
    fireEvent.change(screen.getByTestId("metrics-range-select"), { target: { value: "15m" } })
    expect(onRangeChange).toHaveBeenCalledWith("15m")
  })

  it("warns that counts are estimated when the queue records no native metrics", async () => {
    mocked.getQueueMetrics.mockResolvedValue({
      status: "ok",
      data: metrics({ nativeQueues: 0, totalQueues: 1 }),
    })
    renderOverview()
    expect(await screen.findByTestId("metrics-estimate-notice")).toHaveTextContent(
      /does not record BullMQ metrics/,
    )
  })

  it("shows no estimate warning for a queue with native metrics, and empty-state top tables", async () => {
    mocked.getQueueMetrics.mockResolvedValue({ status: "ok", data: metrics() })
    renderOverview()
    await screen.findByTestId("overview-dashboard")
    expect(screen.queryByTestId("metrics-estimate-notice")).not.toBeInTheDocument()
    expect(screen.getByTestId("slowest-jobs")).toHaveTextContent(
      "No completed jobs in this time range",
    )
    expect(screen.getByTestId("failing-job-types")).toHaveTextContent(
      "No failed jobs in this time range",
    )
  })

  it("lists the slowest jobs and the most failing job types", async () => {
    mocked.getQueueMetrics.mockResolvedValue({
      status: "ok",
      data: metrics({
        sampledJobs: 42,
        minProcessingTimeMs: 100,
        maxProcessingTimeMs: 90_000,
        slowestJobs: [
          {
            queueName: "emails",
            jobId: "17",
            name: "render-digest",
            processingTimeMs: 90_000,
            finishedOn: 1,
          },
        ],
        failingJobTypes: [
          {
            queueName: "emails",
            name: "send-broken",
            count: 5,
            lastFailedAt: 1,
            lastReason: "Connection refused",
          },
        ],
      }),
    })
    renderOverview()
    const slow = await screen.findByTestId("slowest-jobs")
    expect(slow).toHaveTextContent("render-digest")
    expect(slow).toHaveTextContent("emails #17")
    expect(slow).toHaveTextContent("among the last 42 completed")
    const failing = screen.getByTestId("failing-job-types")
    expect(failing).toHaveTextContent("send-broken")
    expect(failing).toHaveTextContent("5 failed")
    expect(failing).toHaveTextContent("Connection refused")
  })
})
