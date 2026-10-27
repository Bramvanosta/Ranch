import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    listFailureCauses: vi.fn(),
    listFailedJobs: vi.fn(),
    bulkRetryJobs: vi.fn(),
    bulkRemoveJobs: vi.fn(),
    getConnectionLockState: vi.fn(),
    unlockConnection: vi.fn(),
    lockConnection: vi.fn(),
  },
}))

import { toast } from "sonner"
import { commands, type FailureCause } from "@/lib/tauri/bindings"

import { FailureCausesView } from "./FailureCausesView"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>
const queueRef = { prefix: "bull", name: "emails" }

function cause(signature: string, ids: string[], extra: Partial<FailureCause> = {}): FailureCause {
  return {
    signature,
    sample: signature,
    count: ids.length,
    firstAt: Date.now() - 3_600_000,
    lastAt: Date.now() - 60_000,
    jobNames: ["send-email"],
    queueNames: ["emails"],
    trend: Array(24).fill(0),
    jobs: ids.map((id) => ({ queue: queueRef, id })),
    ...extra,
  }
}

function renderView() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const onOpenJob = vi.fn()
  const Wrap = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
  render(
    <Wrap>
      <FailureCausesView
        connectionId="c1"
        queueRef={queueRef}
        toggle={<span>toggle</span>}
        onOpenJob={onOpenJob}
      />
    </Wrap>,
  )
  return { onOpenJob }
}

beforeEach(() => {
  mocked.listFailureCauses.mockResolvedValue({
    status: "ok",
    data: {
      causes: [cause("connect ETIMEDOUT <ip>", ["3", "2", "1"]), cause("", ["7"])],
      scanned: 4,
      scanCapped: false,
      totalFailed: 4,
      unreadableQueues: [],
    },
  })
  mocked.getConnectionLockState.mockResolvedValue({ status: "ok", data: "unlocked" })
})

afterEach(() => vi.resetAllMocks())

describe("FailureCausesView", () => {
  it("shows one folding group per cause, most frequent first", async () => {
    renderView()
    const groups = await screen.findAllByTestId("failure-cause")
    expect(groups).toHaveLength(2)
    expect(screen.getAllByTestId("failure-cause-count").map((n) => n.textContent)).toEqual([
      "3",
      "1",
    ])
    expect(screen.getAllByTestId("failure-cause-signature")[1]).toHaveTextContent(
      "No failure reason recorded",
    )
    expect(screen.getByTestId("failure-causes-summary")).toHaveTextContent(
      "2 causes · 4 failed jobs",
    )
    expect(mocked.listFailureCauses).toHaveBeenCalledWith({ connectionId: "c1", queue: "emails" })
  })

  it("the open group lists its jobs, and a job opens", async () => {
    mocked.listFailedJobs.mockResolvedValue({
      status: "ok",
      data: {
        jobs: [
          {
            jobRef: { queue: queueRef, id: "3" },
            name: "send-email",
            failedReason: "connect ETIMEDOUT 10.0.4.12:5432",
            finishedOn: Date.now(),
            timestamp: Date.now(),
          },
        ],
        total: 3,
        totalFailed: 4,
        queues: [],
        types: [],
        scanned: 4,
        scanCapped: false,
        unreadableQueues: [],
      },
    })
    const { onOpenJob } = renderView()
    const [first] = await screen.findAllByTestId("failure-cause-toggle")
    expect(first).toHaveAttribute("aria-expanded", "false")
    fireEvent.click(first)
    expect(first).toHaveAttribute("aria-expanded", "true")
    const job = await screen.findByTestId("failure-cause-job")
    expect(mocked.listFailedJobs).toHaveBeenCalledWith(
      expect.objectContaining({ queue: "emails", signature: "connect ETIMEDOUT <ip>" }),
    )
    expect(screen.getByTestId("failure-cause-jobs")).toHaveTextContent("1 of 3 shown")
    fireEvent.click(job)
    expect(onOpenJob).toHaveBeenCalledWith(expect.objectContaining({ name: "send-email" }))
  })

  it("Retry N goes through a sheet, then retries the jobs of the cause", async () => {
    mocked.bulkRetryJobs.mockResolvedValue({
      status: "ok",
      data: { action: "retry", total: 3, succeeded: 3, failed: [], cancelled: false },
    })
    renderView()
    const [retry] = await screen.findAllByTestId("failure-cause-retry")
    expect(retry).toHaveTextContent("Retry 3")
    fireEvent.click(retry)
    const confirm = await screen.findByTestId("failure-cause-confirm")
    expect(confirm).toHaveTextContent("Retry 3 jobs")
    expect(mocked.bulkRetryJobs).not.toHaveBeenCalled()
    await act(async () => {
      fireEvent.click(confirm)
    })
    await waitFor(() =>
      expect(mocked.bulkRetryJobs).toHaveBeenCalledWith({
        connectionId: "c1",
        queueRef,
        jobIds: ["3", "2", "1"],
      }),
    )
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("Retried 3 jobs"))
  })

  it("on a protected connection, Remove N asks for the queue name and unlocks once", async () => {
    mocked.getConnectionLockState.mockResolvedValue({ status: "ok", data: "locked" })
    mocked.unlockConnection.mockResolvedValue({ status: "ok", data: "unlocked" })
    mocked.lockConnection.mockResolvedValue({ status: "ok", data: "locked" })
    mocked.bulkRemoveJobs.mockResolvedValue({
      status: "ok",
      data: { action: "remove", total: 1, succeeded: 1, failed: [], cancelled: false },
    })
    renderView()
    await waitFor(() => expect(mocked.getConnectionLockState).toHaveBeenCalled())
    const removes = await screen.findAllByTestId("failure-cause-remove")
    fireEvent.click(removes[1])
    const confirm = await screen.findByTestId("failure-cause-confirm")
    await waitFor(() => expect(screen.getByTestId("confirm-sheet-name")).toBeInTheDocument())
    expect(confirm).toBeDisabled()
    fireEvent.change(screen.getByTestId("confirm-sheet-name"), { target: { value: "emails" } })
    await act(async () => {
      fireEvent.click(confirm)
    })
    await waitFor(() => expect(mocked.bulkRemoveJobs).toHaveBeenCalledTimes(1))
    expect(mocked.unlockConnection).toHaveBeenCalledWith({ connectionId: "c1" })
    await waitFor(() => expect(mocked.lockConnection).toHaveBeenCalledWith({ connectionId: "c1" }))
  })
})
