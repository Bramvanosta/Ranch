import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    listQueues: vi.fn(),
    listCronJobs: vi.fn(),
    scheduleRunsWithin: vi.fn(),
    listAuditLogEntries: vi.fn(),
    listRecentJobs: vi.fn(),
    addJob: vi.fn(),
    removeScheduler: vi.fn(),
    getConnectionLockState: vi.fn(),
    unlockConnection: vi.fn(),
    lockConnection: vi.fn(),
  },
}))
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
  Link: ({ children }: { children: ReactNode }) => <a href="#x">{children}</a>,
}))

import { type CronJob, commands } from "@/lib/tauri/bindings"

import { SchedulerInspector } from "./SchedulerInspector"
import { SchedulersView } from "./SchedulersView"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>
const queueRef = { prefix: "bull", name: "emails" }
const HOUR = 3_600_000

function cron(key: string, extra: Partial<CronJob> = {}): CronJob {
  return {
    key,
    name: `job-${key}`,
    pattern: "0 9 * * *",
    every: null,
    tz: null,
    offset: null,
    iterationCount: 3,
    nextRun: Date.now() + 2 * HOUR,
    endDate: null,
    template: '{"kind":"summary"}',
    paused: false,
    ...extra,
  }
}

function wrap(children: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={client}>{children}</QueryClientProvider>)
}

beforeEach(() => {
  mocked.listQueues.mockResolvedValue({
    status: "ok",
    data: [
      {
        queueRef,
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
      },
    ],
  })
  mocked.listCronJobs.mockResolvedValue({
    status: "ok",
    data: [
      cron("daily-report"),
      cron("health-ping", { pattern: null, every: 15 * 60_000, paused: true }),
    ],
  })
  mocked.scheduleRunsWithin.mockResolvedValue({
    status: "ok",
    data: { nextRunsMs: [Date.now() + 2 * HOUR, Date.now() + 26 * HOUR].slice(0, 1) },
  })
  mocked.listAuditLogEntries.mockResolvedValue({
    status: "ok",
    data: [
      {
        id: "a1",
        createdAt: new Date(Date.now() - 2 * HOUR).toISOString(),
        source: "agent",
        operatorId: "me",
        agentClient: "claude-code",
        agentTransport: null,
        action: "pauseScheduler",
        target: { kind: "scheduler", data: { queueName: "emails", schedulerKey: "health-ping" } },
        outcome: "success",
        outcomeReason: null,
        connectionId: "c1",
        connectionName: "local",
        envColor: null,
        groupName: null,
        environmentName: null,
        approvalId: null,
      },
    ],
  })
  mocked.listRecentJobs.mockResolvedValue({
    status: "ok",
    data: { jobs: [], unreadableQueues: [] },
  })
  mocked.getConnectionLockState.mockResolvedValue({ status: "ok", data: "unlocked" })
})
afterEach(() => vi.resetAllMocks())

describe("SchedulersView", () => {
  it("a lane per scheduler: dots for the runs, a paused one hatched with who paused it", async () => {
    wrap(<SchedulersView connectionId="c1" selected={null} onSelect={vi.fn()} />)
    await waitFor(() => expect(screen.getAllByTestId("next-day-lane")).toHaveLength(2))
    expect(screen.getAllByTestId("next-day-dot")).toHaveLength(1)
    await waitFor(() =>
      expect(screen.getByTestId("next-day-paused")).toHaveTextContent("paused by Claude Code"),
    )
    // The strip asks the backend for the next 24 hours of the running one only.
    expect(mocked.scheduleRunsWithin).toHaveBeenCalledTimes(1)
    expect(mocked.scheduleRunsWithin).toHaveBeenCalledWith(
      expect.objectContaining({
        horizonMs: 24 * HOUR,
        schedule: { kind: "pattern", pattern: "0 9 * * *", tz: null },
      }),
    )
  })

  it("the table: schedule in plain language, next run, state; a row opens", async () => {
    const onSelect = vi.fn()
    wrap(<SchedulersView connectionId="c1" selected={null} onSelect={onSelect} />)
    const row = await screen.findByTestId("scheduler-row-daily-report")
    expect(row).toHaveTextContent("daily at 09:00")
    expect(row).toHaveTextContent("in 2 h 0 min")
    expect(row).toHaveTextContent("Active")
    expect(screen.getByTestId("scheduler-row-health-ping")).toHaveTextContent("Paused")
    fireEvent.click(row)
    expect(onSelect).toHaveBeenCalledWith(
      expect.objectContaining({ scheduler: expect.objectContaining({ key: "daily-report" }) }),
    )
  })
})

describe("SchedulerInspector", () => {
  it("next five runs, the template, and Run once now adds the template's job", async () => {
    mocked.scheduleRunsWithin.mockResolvedValue({
      status: "ok",
      data: { nextRunsMs: [1, 2, 3, 4, 5].map((d) => Date.now() + d * 24 * HOUR) },
    })
    mocked.addJob.mockResolvedValue({ status: "ok", data: { jobId: "77", state: "waiting" } })
    wrap(
      <SchedulerInspector
        connectionId="c1"
        queueRef={queueRef}
        scheduler={cron("daily-report")}
        recentJobs={[]}
        onClose={vi.fn()}
      />,
    )
    await waitFor(() =>
      expect(screen.getByTestId("scheduler-next-runs").querySelectorAll("li")).toHaveLength(5),
    )
    expect(screen.getByRole("figure", { name: "Job template" })).toHaveTextContent(
      '"kind": "summary"',
    )
    fireEvent.click(screen.getByTestId("scheduler-run-now"))
    fireEvent.click(await screen.findByTestId("scheduler-confirm-action"))
    await waitFor(() =>
      expect(mocked.addJob).toHaveBeenCalledWith({
        connectionId: "c1",
        queueRef,
        job: { name: "job-daily-report", data: '{"kind":"summary"}' },
      }),
    )
  })

  it("Remove on a protected connection asks for the queue name, under a one-shot unlock", async () => {
    mocked.getConnectionLockState.mockResolvedValue({ status: "ok", data: "locked" })
    mocked.unlockConnection.mockResolvedValue({ status: "ok", data: "unlocked" })
    mocked.lockConnection.mockResolvedValue({ status: "ok", data: "locked" })
    mocked.removeScheduler.mockResolvedValue({
      status: "ok",
      data: { key: "daily-report", paused: false },
    })
    const onClose = vi.fn()
    wrap(
      <SchedulerInspector
        connectionId="c1"
        queueRef={queueRef}
        scheduler={cron("daily-report")}
        recentJobs={[]}
        onClose={onClose}
      />,
    )
    await waitFor(() => expect(mocked.getConnectionLockState).toHaveBeenCalled())
    fireEvent.click(screen.getByTestId("scheduler-remove"))
    const confirm = await screen.findByTestId("scheduler-confirm-action")
    await waitFor(() => expect(screen.getByTestId("confirm-sheet-name")).toBeInTheDocument())
    expect(confirm).toBeDisabled()
    fireEvent.change(screen.getByTestId("confirm-sheet-name"), { target: { value: "emails" } })
    fireEvent.click(confirm)
    await waitFor(() =>
      expect(mocked.removeScheduler).toHaveBeenCalledWith({
        connectionId: "c1",
        queueRef,
        schedulerKey: "daily-report",
      }),
    )
    expect(mocked.unlockConnection).toHaveBeenCalled()
    await waitFor(() => expect(onClose).toHaveBeenCalled())
  })
})
