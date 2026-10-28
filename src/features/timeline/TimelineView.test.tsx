import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { getThroughput: vi.fn(), listAuditLogEntries: vi.fn(), summarizeWindow: vi.fn() },
}))
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="/x">{children}</a>,
}))

import { useAiStore } from "@/lib/ai/aiFeatures"
import { commands } from "@/lib/tauri/bindings"

import { TimelineView } from "./TimelineView"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>
const ok = <T,>(data: T) => ({ status: "ok", data })
const HOUR = 3_600_000
const start = Date.UTC(2026, 9, 27, 0, 0)

const q = (name: string, completed: number[], failed: number[]) => ({
  queueId: `bull:${name}`,
  queueName: name,
  metricsOff: false,
  completed,
  failed,
})

beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    left: 0,
    width: 100,
    top: 0,
    height: 10,
    right: 100,
    bottom: 10,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  })
  mocked.getThroughput.mockResolvedValue(
    ok({
      range: "24h",
      bucketMs: HOUR,
      bucketStartsMs: Array.from({ length: 4 }, (_, i) => start + i * HOUR),
      queues: Array.from({ length: 10 }, (_, i) => q(`queue-${i}`, [i, i, i, i], [0, i % 2, 0, 0])),
      completed: [45, 45, 45, 45],
      failed: [0, 5, 1, 0],
      metricsOffQueues: 0,
    }),
  )
  mocked.listAuditLogEntries.mockResolvedValue(
    ok([
      {
        id: "a1",
        createdAt: new Date(start + HOUR).toISOString(),
        source: "user",
        agentClient: null,
        action: "pauseQueue",
      },
      {
        id: "a2",
        createdAt: new Date(start + 2 * HOUR).toISOString(),
        source: "agent",
        agentClient: "claude-code",
        action: "retryJob",
      },
    ]),
  )
  mocked.summarizeWindow.mockResolvedValue(
    ok({
      fromMs: 0,
      toMs: 0,
      done: 12,
      failed: 9,
      usualDone: 10,
      usualFailed: 1.5,
      waitP95Ms: 4800,
      usualWaitP95Ms: 900,
      causes: [
        {
          signature: "connect ECONNREFUSED <addr>",
          sample: "connect ECONNREFUSED 10.0.0.1:25",
          count: 9,
          jobs: [{ queueId: "bull:emails", queueName: "emails", id: "7" }],
        },
      ],
      scanned: 21,
      scanCapped: false,
    }),
  )
})

afterEach(() => {
  vi.restoreAllMocks()
  vi.resetAllMocks()
  useAiStore.getState().setEnabled(true)
})

function wrap(node: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>)
}

describe("TimelineView", () => {
  it("opens on the moment with the most failures, summarised against the rest", async () => {
    wrap(<TimelineView connectionId="c1" />)
    await waitFor(() => expect(mocked.summarizeWindow).toHaveBeenCalled())
    expect(mocked.summarizeWindow.mock.calls[0][0]).toEqual({
      connectionId: "c1",
      fromMs: start + HOUR,
      toMs: start + 2 * HOUR,
      rangeFromMs: start,
    })
    const summary = await screen.findByTestId("timeline-summary")
    expect(screen.getByTestId("timeline-why")).toHaveTextContent(
      "The moment with the most failures",
    )
    expect(summary).toHaveTextContent("usually 1.5")
    expect(summary).toHaveTextContent("4.8 s")
    expect(screen.getByTestId("timeline-cause")).toHaveTextContent("connect ECONNREFUSED")
    expect(summary).toHaveTextContent("emails 1")
  })

  it("lists every queue with metrics, the failing ones first", async () => {
    wrap(<TimelineView connectionId="c1" />)
    const rows = await screen.findAllByTestId("timeline-queue")
    expect(rows).toHaveLength(10)
    expect(rows[0]).toHaveTextContent("queue-9")
    expect(rows[0]).toHaveTextContent("1 failed")
  })

  it("shows what people and agents did, and plain markers with AI off", async () => {
    wrap(<TimelineView connectionId="c1" />)
    expect(await screen.findByTestId("timeline-marker-audit")).toHaveTextContent("Pause queue")
    expect(screen.getByTestId("timeline-marker-agent")).toHaveTextContent("✦ Retry job")
  })

  it("with AI off, agent actions are plain audit markers", async () => {
    useAiStore.getState().setEnabled(false)
    wrap(<TimelineView connectionId="c1" />)
    await screen.findAllByTestId("timeline-marker-audit")
    expect(screen.queryByTestId("timeline-marker-agent")).not.toBeInTheDocument()
    expect(screen.getByText("What people did")).toBeInTheDocument()
    expect(document.body.textContent).not.toContain("✦")
  })

  it("a dragged moment replaces the one picked, and can be undone", async () => {
    wrap(<TimelineView connectionId="c1" />)
    const chart = await screen.findByTestId("timeline-chart")
    fireEvent.pointerDown(chart, { clientX: 50, pointerId: 1 })
    fireEvent.pointerMove(chart, { clientX: 100, pointerId: 1 })
    fireEvent.pointerUp(chart, { clientX: 100, pointerId: 1 })
    await waitFor(() =>
      expect(mocked.summarizeWindow).toHaveBeenCalledWith({
        connectionId: "c1",
        fromMs: start + 2 * HOUR,
        toMs: start + 4 * HOUR,
        rangeFromMs: start,
      }),
    )
    expect(await screen.findByTestId("timeline-why")).toHaveTextContent("Your selection")
    fireEvent.click(screen.getByRole("button", { name: "Back to the moment picked for you" }))
    expect(await screen.findByTestId("timeline-why")).toHaveTextContent(
      "The moment with the most failures",
    )
  })

  it("says which queues do not record metrics, and how to turn them on", async () => {
    mocked.getThroughput.mockResolvedValue(
      ok({
        range: "24h",
        bucketMs: HOUR,
        bucketStartsMs: [start, start + HOUR],
        queues: [q("cron", [1, 2], [0, 0]), { ...q("import", [], []), metricsOff: true }],
        completed: [1, 2],
        failed: [0, 0],
        metricsOffQueues: 1,
      }),
    )
    wrap(<TimelineView connectionId="c1" />)
    expect(await screen.findByTestId("timeline-metrics-off")).toHaveTextContent(
      "1 of 2 queues does not record BullMQ metrics",
    )
    // Nothing failed: the latest moment is shown.
    expect(await screen.findByTestId("timeline-why")).toHaveTextContent("The latest moment")
  })
})
