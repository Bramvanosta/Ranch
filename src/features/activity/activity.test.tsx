/**
 * Overview pages — jobs per state and what ran last.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, within } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    params,
    to: _to,
    search: _search,
    ...rest
  }: {
    children?: React.ReactNode
    params?: Record<string, string>
    to?: string
    search?: unknown
  } & Record<string, unknown>) => (
    <a href={`#${params?.queueName}/${params?.jobId}`} {...rest}>
      {children}
    </a>
  ),
}))

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listRecentJobs: vi.fn(), getKvSetting: vi.fn() },
}))

import { commands, type Job, type QueueCounters } from "@/lib/tauri/bindings"

import { describeRun, RecentJobsCard } from "./RecentJobsCard"
import { StateTotals } from "./StateTotals"
import { sumCounters } from "./sumCounters"

const mocked = commands as unknown as {
  listRecentJobs: ReturnType<typeof vi.fn>
  getKvSetting: ReturnType<typeof vi.fn>
}

const counters = (over: Partial<QueueCounters>): QueueCounters =>
  ({
    waiting: 0,
    active: 0,
    completed: 0,
    failed: 0,
    delayed: 0,
    prioritized: 0,
    paused: 0,
    ...over,
  }) as QueueCounters

const NOW = 1_800_000_000_000

const job = (over: Partial<Job> & { id: string; queue?: string }): Job =>
  ({
    jobRef: { queue: { prefix: "bull", name: over.queue ?? "emails" }, id: over.id },
    name: "send-email",
    state: "completed",
    data: "{}",
    opts: "{}",
    attemptsMade: 1,
    timestamp: NOW - 60_000,
    processedOn: NOW - 30_000,
    finishedOn: NOW - 28_800,
    delay: null,
    failedReason: null,
    stacktrace: [],
    returnValue: null,
    parentKey: null,
    progress: null,
    ...over,
  }) as Job

function renderCard(queueRef: { prefix: string; name: string } | null) {
  mocked.getKvSetting.mockResolvedValue({ status: "ok", data: null })
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <RecentJobsCard connectionId="c1" queueRef={queueRef} />
    </QueryClientProvider>,
  )
}

afterEach(() => {
  vi.resetAllMocks()
})

describe("sumCounters", () => {
  it("adds the queues up, state by state", () => {
    const totals = sumCounters([
      { counters: counters({ waiting: 3, failed: 1, completed: 1000 }) },
      { counters: counters({ waiting: 2, delayed: 7, completed: 250, paused: 4 }) },
    ])
    expect(totals).toEqual({
      waiting: 5,
      active: 0,
      completed: 1250,
      failed: 1,
      delayed: 7,
      prioritized: 0,
      paused: 4,
    })
  })

  it("is all zeros for no queue, or a queue without counters yet", () => {
    expect(sumCounters([]).waiting).toBe(0)
    expect(sumCounters([{ counters: null as unknown as QueueCounters }]).completed).toBe(0)
  })
})

describe("StateTotals", () => {
  it("shows the five headline states, and the rare ones only when they hold jobs", () => {
    const totals = sumCounters([{ counters: counters({ waiting: 12_345, paused: 2 }) }])
    render(<StateTotals totals={totals} />)
    expect(screen.getByTestId("state-total-waiting")).toHaveTextContent("12,345")
    expect(screen.getByTestId("state-total-paused")).toHaveTextContent("2")
    expect(screen.queryByTestId("state-total-prioritized")).not.toBeInTheDocument()
    for (const s of ["active", "completed", "failed", "delayed"]) {
      expect(screen.getByTestId(`state-total-${s}`)).toBeInTheDocument()
    }
  })

  it("says that completed and failed are what the queue keeps", () => {
    render(<StateTotals totals={sumCounters([{ counters: counters({ completed: 1000 }) }])} />)
    const tile = screen.getByTestId("state-total-completed").parentElement as HTMLElement
    expect(tile).toHaveTextContent("kept")
    expect(tile.getAttribute("title")).toMatch(/removeOnComplete/)
    const waiting = screen.getByTestId("state-total-waiting").parentElement as HTMLElement
    expect(waiting).not.toHaveTextContent("kept")
  })

  it("lets the page decide where a tile leads", () => {
    render(
      <StateTotals
        totals={sumCounters([{ counters: counters({ failed: 3 }) }])}
        linkFor={(state, tile) => (state === "failed" ? <a href="#failed">{tile}</a> : null)}
      />,
    )
    expect(screen.getByTestId("state-total-failed").closest("a")).toHaveAttribute("href", "#failed")
    expect(screen.getByTestId("state-total-waiting").closest("a")).toBeNull()
  })
})

describe("describeRun", () => {
  it("tells how long a job ran, or has been running", () => {
    expect(describeRun(job({ id: "1" }), NOW)).toBe("ran 1.20 s")
    expect(
      describeRun(
        job({ id: "2", state: "active", processedOn: NOW - 14_000, finishedOn: null }),
        NOW,
      ),
    ).toBe("for 14.0 s")
    expect(describeRun(job({ id: "3", processedOn: null }), NOW)).toBeNull()
  })
})

describe("RecentJobsCard", () => {
  it("lists the jobs in the order the backend chose, each one opening its job", async () => {
    mocked.listRecentJobs.mockResolvedValue({
      status: "ok",
      data: {
        jobs: [
          job({ id: "9", state: "active", finishedOn: null }),
          job({
            id: "8",
            queue: "reports",
            state: "failed",
            failedReason: "Error: connect ETIMEDOUT 10.0.4.12:5432\n    at TCPConnectWrap…",
          }),
          job({ id: "7" }),
        ],
        unreadableQueues: [],
      },
    })
    renderCard(null)
    const rows = await screen.findAllByTestId("recent-job-row")
    expect(rows).toHaveLength(3)
    expect(rows[0]).toHaveTextContent("Running")
    expect(rows[0]).toHaveTextContent("now")
    expect(rows[1]).toHaveTextContent("Failed")
    expect(rows[1]).toHaveTextContent("connect ETIMEDOUT")
    expect(rows[1]).not.toHaveTextContent("TCPConnectWrap")
    // Across queues, the queue is part of the identity.
    expect(rows[1]).toHaveTextContent("reports #8")
    expect(rows[1]).toHaveAttribute("href", "#reports/8")
    expect(mocked.listRecentJobs).toHaveBeenCalledWith({
      connectionId: "c1",
      queueRef: null,
      limit: 10,
    })
  })

  it("does not repeat the queue name on the queue's own page", async () => {
    mocked.listRecentJobs.mockResolvedValue({
      status: "ok",
      data: { jobs: [job({ id: "7" })], unreadableQueues: [] },
    })
    renderCard({ prefix: "bull", name: "emails" })
    const row = await screen.findByTestId("recent-job-row")
    expect(within(row).getByText("#7")).toBeInTheDocument()
    expect(row).not.toHaveTextContent("emails #7")
  })

  it("says so when nothing ran, names the queues it could not read, and shows an error", async () => {
    mocked.listRecentJobs.mockResolvedValue({
      status: "ok",
      data: { jobs: [], unreadableQueues: ["legacy"] },
    })
    const first = renderCard(null)
    expect(await screen.findByText(/Nothing is running/)).toBeInTheDocument()
    expect(screen.getByTestId("recent-jobs-unreadable")).toHaveTextContent("legacy")
    first.unmount()

    mocked.listRecentJobs.mockResolvedValue({
      status: "error",
      // A kind the hook does not retry: the error shows at once.
      error: { kind: "schemaNotRecognized", message: "NOAUTH" },
    })
    renderCard(null)
    expect(await screen.findByTestId("recent-jobs-error")).toHaveTextContent("NOAUTH")
  })
})
