import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listFailedJobs: vi.fn() },
}))
// The list and the bulk bar have their own suites; here: what the view asks
// the backend, and what it hands to the list.
vi.mock("@/features/jobs/JobList", () => ({
  JobList: ({
    jobs,
    showQueueBadge,
    crossQueueNote,
  }: {
    jobs: { jobRef: { id: string; queueName: string } }[]
    showQueueBadge: boolean
    crossQueueNote: string | null
  }) => (
    <div data-testid="job-list" data-badge={String(showQueueBadge)}>
      {jobs.map((j) => (
        <span key={`${j.jobRef.queueName}-${j.jobRef.id}`}>
          {j.jobRef.queueName}#{j.jobRef.id}
        </span>
      ))}
      {crossQueueNote ? <p data-testid="scan-note">{crossQueueNote}</p> : null}
    </div>
  ),
}))
vi.mock("@/features/jobs/BulkActionBar", () => ({
  BulkActionBar: () => <div data-testid="bulk-bar" />,
}))

import { EMPTY_FAILED_FILTER, type FailedJobsFilter } from "@/data/jobs/failures"
import { commands } from "@/lib/tauri/bindings"
import { FailedJobsView } from "./FailedJobsView"

const mocked = commands as unknown as { listFailedJobs: ReturnType<typeof vi.fn> }

const job = (queue: string, id: string) => ({
  jobRef: { id, queueId: `bull:${queue}`, queueName: queue },
})

const page = (over: Record<string, unknown> = {}) => ({
  jobs: [job("emails", "3"), job("images", "7"), job("emails", "2")],
  total: 3,
  totalFailed: 3,
  queues: [
    { queueId: "bull:emails", queueName: "emails", failed: 2 },
    { queueId: "bull:images", queueName: "images", failed: 1 },
  ],
  types: [
    {
      queueId: "bull:emails",
      queueName: "emails",
      name: "send-email",
      count: 2,
      lastFailedAt: 1,
      lastReason: "ETIMEDOUT",
    },
  ],
  scanned: 3,
  scanCapped: false,
  unreadableQueues: [],
  ...over,
})

afterEach(() => vi.resetAllMocks())

function renderView(filter: FailedJobsFilter = EMPTY_FAILED_FILTER, pageIndex = 0) {
  const onFilterChange = vi.fn()
  const onPageChange = vi.fn()
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <FailedJobsView
        connectionId="c1"
        filter={filter}
        page={pageIndex}
        onFilterChange={onFilterChange}
        onPageChange={onPageChange}
      />
    </QueryClientProvider>,
  )
  return { onFilterChange, onPageChange }
}

describe("FailedJobsView", () => {
  it("shows the failures of every queue in one list, with queue badges and per-queue counts", async () => {
    mocked.listFailedJobs.mockResolvedValue({ status: "ok", data: page() })
    renderView()

    const list = await screen.findByTestId("job-list")
    expect(list).toHaveTextContent("emails#3")
    expect(list).toHaveTextContent("images#7")
    expect(list.dataset.badge).toBe("true")
    expect(screen.getByTestId("failed-jobs-count")).toHaveTextContent("3 failed")
    expect(screen.getByRole("button", { name: /All queues\s*3/ })).toHaveAttribute(
      "aria-pressed",
      "true",
    )
    // Anchored: the "send-email · emails · 2" job-type button also contains "emails 2".
    expect(screen.getByRole("button", { name: /^emails\s*2$/ })).toBeInTheDocument()
    expect(screen.getByTestId("bulk-bar")).toBeInTheDocument()
    expect(mocked.listFailedJobs).toHaveBeenCalledWith({
      connectionId: "c1",
      offset: 0,
      limit: 50,
      queue: null,
      name: null,
      reason: null,
    })
  })

  it("queue chips and job types are filters; clicking the active one clears it", async () => {
    mocked.listFailedJobs.mockResolvedValue({ status: "ok", data: page() })
    const { onFilterChange } = renderView()
    await screen.findByTestId("job-list")

    fireEvent.click(screen.getByRole("button", { name: /images\s*1/ }))
    expect(onFilterChange).toHaveBeenLastCalledWith({ queue: "images", name: "", reason: "" })

    fireEvent.click(screen.getByTestId("failed-type-emails-send-email"))
    expect(onFilterChange).toHaveBeenLastCalledWith({
      queue: "emails",
      name: "send-email",
      reason: "",
    })
  })

  it("passes the filters and the page to the backend", async () => {
    mocked.listFailedJobs.mockResolvedValue({ status: "ok", data: page({ total: 120 }) })
    const { onPageChange } = renderView(
      { queue: "emails", name: "send-email", reason: "timeout" },
      1,
    )
    await screen.findByTestId("job-list")

    expect(mocked.listFailedJobs).toHaveBeenCalledWith({
      connectionId: "c1",
      offset: 50,
      limit: 50,
      queue: "emails",
      name: "send-email",
      reason: "timeout",
    })
    expect(screen.getByTestId("failed-jobs-count")).toHaveTextContent("120 matching")
    expect(screen.getByText("2 / 3")).toBeInTheDocument()
    fireEvent.click(screen.getByRole("button", { name: "Older" }))
    expect(onPageChange).toHaveBeenCalledWith(2)
  })

  it("debounces the reason field", async () => {
    mocked.listFailedJobs.mockResolvedValue({ status: "ok", data: page() })
    const { onFilterChange } = renderView()
    await screen.findByTestId("job-list")

    fireEvent.change(screen.getByLabelText("Filter by failure reason"), {
      target: { value: "ETIMEDOUT" },
    })
    expect(onFilterChange).not.toHaveBeenCalled()
    await waitFor(() =>
      expect(onFilterChange).toHaveBeenCalledWith({ queue: "", name: "", reason: "ETIMEDOUT" }),
    )
  })

  it("celebrates an empty connection, and explains a capped scan or an unreadable queue", async () => {
    mocked.listFailedJobs.mockResolvedValue({
      status: "ok",
      data: page({ jobs: [], total: 0, totalFailed: 0, queues: [], types: [] }),
    })
    renderView()
    expect(await screen.findByTestId("failed-jobs-empty")).toHaveTextContent(/Nothing has failed/)
  })

  it("says when only the most recent failures were looked at", async () => {
    mocked.listFailedJobs.mockResolvedValue({
      status: "ok",
      data: page({ scanCapped: true, scanned: 3000, unreadableQueues: ["legacy"] }),
    })
    renderView()
    expect(await screen.findByTestId("scan-note")).toHaveTextContent(/3,000 most recent failures/)
    expect(screen.getByRole("note")).toHaveTextContent("Could not read: legacy.")
  })

  it("surfaces a backend error", { timeout: 10_000 }, async () => {
    mocked.listFailedJobs.mockResolvedValue({
      status: "error",
      error: { kind: "redis", message: "connection refused" },
    })
    renderView()
    // The hook retries transient errors twice before giving up.
    expect(
      await screen.findByTestId("failed-jobs-error", {}, { timeout: 8_000 }),
    ).toHaveTextContent("connection refused")
  })
})
