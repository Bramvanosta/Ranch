/**
 * The Jobs tab sends its filter, sort and page size to the
 * server in queue scope, and keeps the client-side filter for
 * the cross-queue scope.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({ toast: vi.fn() }))
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    listJobs: vi.fn(),
    getKvSetting: vi.fn(),
    setKvSetting: vi.fn(),
  },
}))
vi.mock("@/features/queues/useQueues", () => ({
  useQueues: () => ({
    data: [
      { queueRef: { prefix: "bull", name: "emails" } },
      { queueRef: { prefix: "bull", name: "images" } },
    ],
  }),
}))

import { commands, type Job, type ListJobsOutput, type QueueCounters } from "@/lib/tauri/bindings"

import { FILTER_DEBOUNCE_MS, JobsPanel, type JobsPanelProps } from "./JobsPanel"

const mocked = commands as unknown as {
  listJobs: ReturnType<typeof vi.fn>
  getKvSetting: ReturnType<typeof vi.fn>
  setKvSetting: ReturnType<typeof vi.fn>
}

function makeJob(id: string, name: string, queue = "emails"): Job {
  return {
    jobRef: { queue: { prefix: "bull", name: queue }, id },
    name,
    data: "{}",
    opts: "{}",
    state: "failed",
    timestamp: 1_000,
    processedOn: null,
    finishedOn: null,
    attemptsMade: 1,
    delay: null,
    failedReason: "boom",
    stacktrace: [],
    returnValue: null,
    parentKey: null,
    progress: null,
  }
}

function output(jobs: Job[], extra: Partial<ListJobsOutput> = {}): ListJobsOutput {
  return {
    jobs,
    total: jobs.length,
    state: "failed",
    hasMore: false,
    scanned: null,
    scanCapped: false,
    ...extra,
  }
}

const counters = { failed: 4200 } as unknown as QueueCounters

const baseProps: JobsPanelProps = {
  connectionId: "c1",
  queueRef: { prefix: "bull", name: "emails" },
  activeState: "failed",
  page: 0,
  scope: "queue",
  filter: { name: "", error: "" },
  sort: null,
  pageSize: 50,
  counters,
  onFilterChange: vi.fn(),
  onScopeChange: vi.fn(),
  onPageChange: vi.fn(),
  onSortChange: vi.fn(),
  onPageSizeChange: vi.fn(),
}

function renderPanel(props: Partial<JobsPanelProps> = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
  const view = render(<JobsPanel {...baseProps} {...props} />, { wrapper: Wrapper })
  return {
    ...view,
    rerenderPanel: (next: Partial<JobsPanelProps>) =>
      view.rerender(<JobsPanel {...baseProps} {...next} />),
  }
}

beforeEach(() => {
  mocked.getKvSetting.mockResolvedValue({ status: "ok", data: "1" })
  mocked.setKvSetting.mockResolvedValue({ status: "ok", data: null })
})

afterEach(() => {
  vi.useRealTimers()
  vi.resetAllMocks()
})

describe("JobsPanel — queue scope (server-side)", () => {
  it("sends the filter, sort and page size to list_jobs", async () => {
    mocked.listJobs.mockResolvedValue({
      status: "ok",
      data: output([makeJob("7", "needle-export")], { total: 1, scanned: 4200 }),
    })
    renderPanel({
      filter: { name: "needle", error: "timeout" },
      sort: { field: "duration", descending: true },
      pageSize: 100,
      page: 2,
    })
    await waitFor(() => expect(mocked.listJobs).toHaveBeenCalledTimes(1))
    expect(mocked.listJobs).toHaveBeenCalledWith({
      connectionId: "c1",
      queueRef: { prefix: "bull", name: "emails" },
      state: "failed",
      offset: 200,
      limit: 100,
      newestFirst: true,
      search: "needle",
      reason: "timeout",
      sort: { field: "duration", descending: true },
    })
    // Matching jobs of all the jobs in the state — not "1 of 1".
    expect(await screen.findByText("1 of 4,200 jobs")).toBeInTheDocument()
    // Single-queue scope never fans out to the other queues.
    expect(mocked.listJobs.mock.calls.every(([a]) => a.queueRef.name === "emails")).toBe(true)
  })

  it("waits for typing to settle before querying again", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    mocked.listJobs.mockResolvedValue({ status: "ok", data: output([makeJob("1", "a")]) })
    const { rerenderPanel } = renderPanel()
    await waitFor(() => expect(mocked.listJobs).toHaveBeenCalledTimes(1))

    rerenderPanel({ filter: { name: "n", error: "" } })
    rerenderPanel({ filter: { name: "ne", error: "" } })
    rerenderPanel({ filter: { name: "nee", error: "" } })
    await act(async () => {
      await vi.advanceTimersByTimeAsync(FILTER_DEBOUNCE_MS - 50)
    })
    expect(mocked.listJobs).toHaveBeenCalledTimes(1)

    await act(async () => {
      await vi.advanceTimersByTimeAsync(100)
    })
    await waitFor(() => expect(mocked.listJobs).toHaveBeenCalledTimes(2))
    expect(mocked.listJobs).toHaveBeenLastCalledWith(expect.objectContaining({ search: "nee" }))
  })

  it("says it is searching while the scan runs", async () => {
    let resolve: (v: unknown) => void = () => {}
    mocked.listJobs.mockReturnValue(
      new Promise((r) => {
        resolve = r
      }),
    )
    renderPanel({ filter: { name: "needle", error: "" } })
    expect(await screen.findByText("Searching…")).toBeInTheDocument()
    await act(async () => {
      resolve({ status: "ok", data: output([makeJob("7", "needle-export")], { total: 1 }) })
    })
    expect(await screen.findByText("1 of 4,200 jobs")).toBeInTheDocument()
  })

  it("says when the search could not look at every job", async () => {
    mocked.listJobs.mockResolvedValue({
      status: "ok",
      data: output([makeJob("1", "a")], { total: 1, scanned: 10_000, scanCapped: true }),
    })
    renderPanel({ filter: { name: "a", error: "" } })
    expect(await screen.findByTestId("job-filter-scan-note")).toHaveTextContent(
      /newest 10,000 failed jobs/,
    )
  })

  it("shows the query error instead of an empty list", async () => {
    mocked.listJobs.mockResolvedValue({
      status: "error",
      // Non-retryable kind: the hook surfaces it without its backoff.
      error: { kind: "schemaNotRecognized", message: "Schema 7.x not supported", docUrl: null },
    })
    renderPanel()
    expect(await screen.findByRole("alert")).toHaveTextContent(/Schema 7.x not supported/)
  })
})

describe("JobsPanel — cross-queue scope (client-side)", () => {
  it("fetches the first page of every queue, unfiltered, and filters in the browser", async () => {
    mocked.listJobs.mockImplementation(async (args: { queueRef: { name: string } }) => ({
      status: "ok",
      data:
        args.queueRef.name === "emails"
          ? output([makeJob("1", "send-email"), makeJob("77", "digest")], { total: 2 })
          : output([makeJob("2", "resize", "images")], { total: 1 }),
    }))
    renderPanel({ scope: "cross", filter: { name: "77", error: "" } })
    // One fetch per queue, none carrying the filter.
    await waitFor(() => expect(mocked.listJobs).toHaveBeenCalledTimes(2))
    for (const [args] of mocked.listJobs.mock.calls) {
      expect(args.search ?? null).toBeNull()
    }
    // The name segment also matches the job id, as the server does.
    expect(await screen.findByText("1 of 3 jobs")).toBeInTheDocument()
    expect(screen.queryByTestId("job-list-page-size")).not.toBeInTheDocument()
  })
})
