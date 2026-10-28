/**
 * The Jobs tab sends its filter, sort and page size to the
 * server in queue scope, and keeps the client-side filter for
 * the cross-queue scope.
 */
import { act, screen, waitFor } from "@testing-library/react"
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
vi.mock("@/data/queues/queries", () => ({
  useQueues: () => ({
    data: [
      { id: "bull:emails", name: "emails" },
      { id: "bull:images", name: "images" },
    ],
  }),
}))

import { commands, type Job, type ListJobsOutput } from "@/lib/tauri/bindings"
import { aJob, aQueue, aQueueId } from "@/test/factories"
import { renderWithProviders } from "@/test/renderWithProviders"
import { err, ok } from "@/test/results"

import { FILTER_DEBOUNCE_MS, JobsPanel, type JobsPanelProps } from "./JobsPanel"

const listJobs = vi.mocked(commands.listJobs)
const getKvSetting = vi.mocked(commands.getKvSetting)
const setKvSetting = vi.mocked(commands.setKvSetting)

function failedJob(id: string, name: string, queue = "emails"): Job {
  return aJob({
    id,
    name,
    queue,
    state: "failed",
    timestamp: 1_000,
    attemptsMade: 1,
    failedReason: "boom",
  })
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

const counters = { ...aQueue().counts, failed: 4200 }

const baseProps: JobsPanelProps = {
  connectionId: "c1",
  queueId: aQueueId(),
  activeState: "failed",
  page: 0,
  scope: "queue",
  filter: { name: "", error: "" },
  sort: null,
  pageSize: 50,
  counts: counters,
  onFilterChange: vi.fn(),
  onScopeChange: vi.fn(),
  onPageChange: vi.fn(),
  onSortChange: vi.fn(),
  onPageSizeChange: vi.fn(),
}

function renderPanel(props: Partial<JobsPanelProps> = {}) {
  const view = renderWithProviders(<JobsPanel {...baseProps} {...props} />)
  return {
    ...view,
    rerenderPanel: (next: Partial<JobsPanelProps>) =>
      view.rerender(<JobsPanel {...baseProps} {...next} />),
  }
}

beforeEach(() => {
  getKvSetting.mockResolvedValue(ok("1"))
  setKvSetting.mockResolvedValue(ok(null))
})

afterEach(() => {
  vi.useRealTimers()
  vi.resetAllMocks()
})

describe("JobsPanel: queue scope (server-side)", () => {
  it("sends the filter, sort and page size to list_jobs", async () => {
    listJobs.mockResolvedValue(
      ok(output([failedJob("7", "needle-export")], { total: 1, scanned: 4200 })),
    )
    renderPanel({
      filter: { name: "needle", error: "timeout" },
      sort: { field: "duration", descending: true },
      pageSize: 100,
      page: 2,
    })
    await waitFor(() => expect(listJobs).toHaveBeenCalledTimes(1))
    expect(listJobs).toHaveBeenCalledWith({
      connectionId: "c1",
      queueId: "bull:emails",
      state: "failed",
      offset: 200,
      limit: 100,
      newestFirst: true,
      search: "needle",
      reason: "timeout",
      sort: { field: "duration", descending: true },
    })
    // Matching jobs of all the jobs in the state, not "1 of 1".
    expect(await screen.findByText("1 of 4,200 jobs")).toBeInTheDocument()
    // Single-queue scope never fans out to the other queues.
    expect(listJobs.mock.calls.every(([a]) => a.queueId === "bull:emails")).toBe(true)
  })

  it("waits for typing to settle before querying again", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    listJobs.mockResolvedValue(ok(output([failedJob("1", "a")])))
    const { rerenderPanel } = renderPanel()
    await waitFor(() => expect(listJobs).toHaveBeenCalledTimes(1))

    rerenderPanel({ filter: { name: "n", error: "" } })
    rerenderPanel({ filter: { name: "ne", error: "" } })
    rerenderPanel({ filter: { name: "nee", error: "" } })
    await act(async () => {
      await vi.advanceTimersByTimeAsync(FILTER_DEBOUNCE_MS - 50)
    })
    expect(listJobs).toHaveBeenCalledTimes(1)

    await act(async () => {
      await vi.advanceTimersByTimeAsync(100)
    })
    await waitFor(() => expect(listJobs).toHaveBeenCalledTimes(2))
    expect(listJobs).toHaveBeenLastCalledWith(expect.objectContaining({ search: "nee" }))
  })

  it("says it is searching while the scan runs", async () => {
    let resolve: (v: Awaited<ReturnType<typeof commands.listJobs>>) => void = () => {}
    listJobs.mockReturnValue(
      new Promise((r) => {
        resolve = r
      }),
    )
    renderPanel({ filter: { name: "needle", error: "" } })
    expect(await screen.findByText("Searching…")).toBeInTheDocument()
    await act(async () => {
      resolve(ok(output([failedJob("7", "needle-export")], { total: 1 })))
    })
    expect(await screen.findByText("1 of 4,200 jobs")).toBeInTheDocument()
  })

  it("says when the search could not look at every job", async () => {
    listJobs.mockResolvedValue(
      ok(output([failedJob("1", "a")], { total: 1, scanned: 10_000, scanCapped: true })),
    )
    renderPanel({ filter: { name: "a", error: "" } })
    expect(await screen.findByTestId("job-filter-scan-note")).toHaveTextContent(
      /newest 10,000 failed jobs/,
    )
  })

  it("shows the query error instead of an empty list", async () => {
    // Non-retryable kind: the hook surfaces it without its backoff.
    listJobs.mockResolvedValue(err("schemaNotRecognized", "Schema 7.x not supported"))
    renderPanel()
    expect(await screen.findByRole("alert")).toHaveTextContent(/Schema 7.x not supported/)
  })
})

describe("JobsPanel: cross-queue scope (client-side)", () => {
  it("fetches the first page of every queue, unfiltered, and filters in the browser", async () => {
    listJobs.mockImplementation(async (args) =>
      ok(
        args.queueId === "bull:emails"
          ? output([failedJob("1", "send-email"), failedJob("77", "digest")], { total: 2 })
          : output([failedJob("2", "resize", "images")], { total: 1 }),
      ),
    )
    renderPanel({ scope: "cross", filter: { name: "77", error: "" } })
    // One fetch per queue, none carrying the filter.
    await waitFor(() => expect(listJobs).toHaveBeenCalledTimes(2))
    for (const [args] of listJobs.mock.calls) {
      expect(args.search ?? null).toBeNull()
    }
    // The name segment also matches the job id, as the server does.
    expect(await screen.findByText("1 of 3 jobs")).toBeInTheDocument()
    expect(screen.queryByTestId("job-list-page-size")).not.toBeInTheDocument()
  })
})
