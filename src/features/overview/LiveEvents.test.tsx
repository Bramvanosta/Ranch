import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    readQueueEvents: vi.fn(),
    listMcpActivity: vi.fn(),
    listPendingApprovals: vi.fn(),
    decidePendingApproval: vi.fn(),
  },
}))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }))
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, ...rest }: { children: ReactNode } & Record<string, unknown>) => (
    <a href="#x" className={rest.className as string}>
      {children}
    </a>
  ),
}))

import { LIVE_EVENTS_MS } from "@/data/events/queries"
import { commands } from "@/lib/tauri/bindings"
import { LiveEvents } from "./LiveEvents"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>

const event = (id: string, kind: string, extra: Record<string, unknown> = {}) => ({
  queueName: "emails",
  streamId: `${id}-0`,
  atMs: Number(id),
  event: kind,
  jobId: "42",
  jobName: "send-email",
  failedReason: null,
  ...extra,
})

function renderPanel() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <LiveEvents connectionId="c1" />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true })
  mocked.listMcpActivity.mockResolvedValue({ status: "ok", data: { events: [], total: 0 } })
  mocked.listPendingApprovals.mockResolvedValue({ status: "ok", data: [] })
})
afterEach(() => {
  vi.useRealTimers()
  vi.resetAllMocks()
})

describe("LiveEvents", () => {
  it("shows the last events, then reads from where it stopped", async () => {
    mocked.readQueueEvents
      .mockResolvedValueOnce({
        status: "ok",
        data: {
          events: [
            event("2000", "failed", { failedReason: "boom\n  at x" }),
            event("1000", "completed"),
          ],
          cursors: [{ queueName: "emails", lastId: "2000-0" }],
          sampled: false,
        },
      })
      .mockResolvedValue({
        status: "ok",
        data: {
          events: [event("3000", "active")],
          cursors: [{ queueName: "emails", lastId: "3000-0" }],
          sampled: true,
        },
      })
    renderPanel()
    await waitFor(() => expect(screen.getAllByTestId("live-event")).toHaveLength(2))
    const first = screen.getAllByTestId("live-event")[0]
    expect(first).toHaveAttribute("data-event", "failed")
    expect(first).toHaveTextContent("send-email")
    expect(first).toHaveTextContent("emails/42")
    expect(first).toHaveTextContent("boom")

    await act(async () => {
      await vi.advanceTimersByTimeAsync(LIVE_EVENTS_MS)
    })
    await waitFor(() => expect(screen.getAllByTestId("live-event")).toHaveLength(3))
    expect(mocked.readQueueEvents).toHaveBeenLastCalledWith({
      connectionId: "c1",
      cursors: [{ queueName: "emails", lastId: "2000-0" }],
      limit: 100,
    })
    expect(screen.getAllByTestId("live-event")[0]).toHaveAttribute("data-event", "active")
    expect(screen.getByTestId("live-events-sampled")).toBeInTheDocument()
  })

  it("Pause freezes the list: no more reads until Resume", async () => {
    mocked.readQueueEvents.mockResolvedValue({
      status: "ok",
      data: { events: [], cursors: [], sampled: false },
    })
    renderPanel()
    await waitFor(() => expect(mocked.readQueueEvents).toHaveBeenCalledTimes(1))
    fireEvent.click(screen.getByTestId("live-events-pause"))
    await act(async () => {
      await vi.advanceTimersByTimeAsync(LIVE_EVENTS_MS * 3)
    })
    expect(mocked.readQueueEvents).toHaveBeenCalledTimes(1)
    fireEvent.click(screen.getByTestId("live-events-pause"))
    await waitFor(() => expect(mocked.readQueueEvents).toHaveBeenCalledTimes(2))
  })

  it("agent calls show with ✦, and a pending approval sits on top", async () => {
    mocked.readQueueEvents.mockResolvedValue({
      status: "ok",
      data: { events: [], cursors: [], sampled: false },
    })
    mocked.listMcpActivity.mockResolvedValue({
      status: "ok",
      data: {
        total: 1,
        events: [
          {
            id: "a1",
            sessionId: "s",
            timestamp: new Date().toISOString(),
            clientHint: "claude-code",
            tool: "retry_job",
            kind: "write",
            connectionId: "c1",
            connectionName: "prod",
            targetJson: null,
            status: "success",
            durationMs: 3,
            error: null,
            approvalId: null,
          },
        ],
      },
    })
    mocked.listPendingApprovals.mockResolvedValue({
      status: "ok",
      data: [
        {
          id: "ap1",
          createdAt: new Date().toISOString(),
          clientHint: "claude-code",
          connectionId: "c1",
          connectionName: "prod",
          groupName: "g",
          environmentName: "Production",
          envColor: "env-production",
          tool: "drain_queue",
          kind: "write",
          argumentsJson: "{}",
          targetPreviewJson: null,
          timeoutSeconds: 120,
        },
      ],
    })
    mocked.decidePendingApproval.mockResolvedValue({ status: "ok", data: "decided" })
    renderPanel()
    expect(await screen.findByTestId("live-event-agent")).toHaveTextContent("retry_job")
    const card = await screen.findByTestId("live-events-approval")
    expect(card).toHaveTextContent("drain_queue")
    expect(card).toHaveTextContent("s left")
    fireEvent.click(screen.getByTestId("live-events-approval-deny"))
    await waitFor(() =>
      expect(mocked.decidePendingApproval).toHaveBeenCalledWith({
        approvalId: "ap1",
        decision: "deny",
        reason: null,
      }),
    )
  })
})
