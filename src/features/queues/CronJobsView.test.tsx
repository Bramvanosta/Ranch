import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    listCronJobs: vi.fn(),
    // CronJobsView now mounts useSchedulerActions hooks
    // for the Pause/Resume per row.
    pauseScheduler: vi.fn(),
    resumeScheduler: vi.fn(),
  },
}))

import { type CronJob, commands } from "@/lib/tauri/bindings"

import { CronJobsView } from "./CronJobsView"

const mocked = commands as unknown as { listCronJobs: ReturnType<typeof vi.fn> }

afterEach(() => {
  vi.resetAllMocks()
})

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

const queueRef = { prefix: "bull", name: "emails" }

function makeCron(overrides: Partial<CronJob> = {}): CronJob {
  return {
    key: "daily-report",
    name: "send-daily-report",
    pattern: "0 9 * * *",
    every: null,
    tz: "Europe/Paris",
    offset: 0,
    iterationCount: 5,
    nextRun: Date.now() + 60_000, // future by default
    endDate: null,
    template: null,
    paused: false,
    ...overrides,
  }
}

describe("CronJobsView", () => {
  it("renders empty state when no schedulers", async () => {
    mocked.listCronJobs.mockResolvedValue({ status: "ok", data: [] })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <CronJobsView connectionId="c1" queueRef={queueRef} />
      </Wrapper>,
    )
    expect(await screen.findByTestId("cron-jobs-empty")).toBeInTheDocument()
    expect(screen.getByText(/No scheduler in this queue/)).toBeInTheDocument()
    // The empty state is also where the first one is created.
    expect(screen.getByTestId("scheduler-new")).toBeEnabled()
  })

  it("renders error state with retry button when fetch fails", async () => {
    mocked.listCronJobs.mockResolvedValue({
      status: "error",
      error: { kind: "schemaNotRecognized", message: "BullMQ 7.x", docUrl: null },
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <CronJobsView connectionId="c1" queueRef={queueRef} />
      </Wrapper>,
    )
    expect(await screen.findByTestId("cron-jobs-error")).toHaveTextContent(/Could not list/)
    expect(screen.getByText(/BullMQ 7.x/)).toBeInTheDocument()
  })

  it("renders both cron-pattern and every-style schedulers in the same table", async () => {
    mocked.listCronJobs.mockResolvedValue({
      status: "ok",
      data: [
        makeCron({ key: "daily-report", pattern: "0 9 * * *", every: null }),
        makeCron({
          key: "every-15m",
          name: "ping-health",
          pattern: null,
          every: 15 * 60 * 1000,
          tz: null,
        }),
      ],
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <CronJobsView connectionId="c1" queueRef={queueRef} />
      </Wrapper>,
    )
    expect(await screen.findByTestId("cron-jobs-view")).toBeInTheDocument()
    expect(screen.getByTestId("cron-row-daily-report")).toHaveTextContent("0 9 * * *")
    expect(screen.getByTestId("cron-row-every-15m")).toHaveTextContent("every 15m")
  })

  it("highlights past-due rows with the warn icon", async () => {
    mocked.listCronJobs.mockResolvedValue({
      status: "ok",
      data: [
        makeCron({ key: "overdue-cron", nextRun: Date.now() - 60_000 /* 1m ago */ }),
        makeCron({ key: "future-cron", nextRun: Date.now() + 60_000 }),
      ],
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <CronJobsView connectionId="c1" queueRef={queueRef} />
      </Wrapper>,
    )
    expect(await screen.findByTestId("cron-row-overdue-cron")).toHaveAttribute(
      "data-past-due",
      "true",
    )
    expect(screen.getByTestId("cron-row-past-due-icon-overdue-cron")).toBeInTheDocument()
    expect(screen.getByTestId("cron-row-future-cron")).toHaveAttribute("data-past-due", "false")
    expect(screen.queryByTestId("cron-row-past-due-icon-future-cron")).not.toBeInTheDocument()
  })

  it("renders em-dash in the TZ column when tz is null", async () => {
    mocked.listCronJobs.mockResolvedValue({
      status: "ok",
      data: [makeCron({ key: "no-tz", tz: null })],
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <CronJobsView connectionId="c1" queueRef={queueRef} />
      </Wrapper>,
    )
    expect(await screen.findByTestId("cron-row-no-tz")).toHaveTextContent("—")
  })

  // ── Pause/Resume per row ────────────────────────────────

  it("renders a Paused badge on paused rows", async () => {
    mocked.listCronJobs.mockResolvedValue({
      status: "ok",
      data: [makeCron({ key: "k1", paused: true })],
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <CronJobsView connectionId="c1" queueRef={queueRef} />
      </Wrapper>,
    )
    expect(await screen.findByTestId("cron-row-paused-badge-k1")).toBeInTheDocument()
    // Row's data-paused attribute also reflects state.
    expect(screen.getByTestId("cron-row-k1")).toHaveAttribute("data-paused", "true")
  })

  it("Pause button click fires pauseScheduler with the row's key", async () => {
    const { act, fireEvent, waitFor } = await import("@testing-library/react")
    mocked.listCronJobs.mockResolvedValue({
      status: "ok",
      data: [makeCron({ key: "running-1", paused: false })],
    })
    const mockedPause = commands.pauseScheduler as unknown as ReturnType<typeof vi.fn>
    mockedPause.mockResolvedValue({
      status: "ok",
      data: { paused: true, alreadyInState: false },
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <CronJobsView connectionId="c1" queueRef={queueRef} />
      </Wrapper>,
    )
    const btn = await screen.findByTestId("cron-row-toggle-running-1")
    expect(btn.textContent).toBe("Pause")
    await act(async () => {
      fireEvent.click(btn)
    })
    await waitFor(() =>
      expect(mockedPause).toHaveBeenCalledWith({
        connectionId: "c1",
        queueRef,
        schedulerKey: "running-1",
      }),
    )
  })

  it("Resume button click fires resumeScheduler when row is paused", async () => {
    const { act, fireEvent, waitFor } = await import("@testing-library/react")
    mocked.listCronJobs.mockResolvedValue({
      status: "ok",
      data: [makeCron({ key: "paused-1", paused: true })],
    })
    const mockedResume = commands.resumeScheduler as unknown as ReturnType<typeof vi.fn>
    mockedResume.mockResolvedValue({
      status: "ok",
      data: { paused: false, alreadyInState: false },
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <CronJobsView connectionId="c1" queueRef={queueRef} />
      </Wrapper>,
    )
    const btn = await screen.findByTestId("cron-row-toggle-paused-1")
    expect(btn.textContent).toBe("Resume")
    await act(async () => {
      fireEvent.click(btn)
    })
    await waitFor(() =>
      expect(mockedResume).toHaveBeenCalledWith({
        connectionId: "c1",
        queueRef,
        schedulerKey: "paused-1",
      }),
    )
  })
})
