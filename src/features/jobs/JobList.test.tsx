import { fireEvent, screen } from "@testing-library/react"
import type { ComponentProps } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listJobs: vi.fn() },
}))

import { commands, type Job, type ListJobsOutput } from "@/lib/tauri/bindings"
import { aJob, aQueueId } from "@/test/factories"
import { renderWithProviders } from "@/test/renderWithProviders"
import { err, ok } from "@/test/results"

import { JobList } from "./JobList"

const listJobs = vi.mocked(commands.listJobs)

afterEach(() => {
  vi.resetAllMocks()
})

function output(jobs: Job[], total = jobs.length, hasMore = false): ListJobsOutput {
  return { jobs, total, state: "waiting", hasMore, scanned: null, scanCapped: false }
}

function jobs(count: number): Job[] {
  return Array.from({ length: count }, (_, i) => aJob({ id: String(i + 1) }))
}

type Props = ComponentProps<typeof JobList>

function list(props: Partial<Props> = {}) {
  return (
    <JobList
      connectionId="c1"
      queueId={aQueueId()}
      state="waiting"
      page={0}
      onPageChange={vi.fn()}
      {...props}
    />
  )
}

describe("JobList", () => {
  it("shows the empty state when total=0", async () => {
    listJobs.mockResolvedValue(ok(output([], 0)))
    renderWithProviders(list())
    expect(await screen.findByText(/No waiting jobs in this queue/)).toBeInTheDocument()
  })

  it("renders an error block when the query fails", async () => {
    listJobs.mockResolvedValue(err("schemaNotRecognized", "Schema 7.x not supported"))
    renderWithProviders(list({ state: "failed" }))
    expect(await screen.findByRole("alert")).toHaveTextContent(/Could not list jobs/)
    expect(screen.getByRole("alert")).toHaveTextContent(/Schema 7.x not supported/)
  })

  it("renders rows + pagination footer when populated", async () => {
    listJobs.mockResolvedValue(ok(output(jobs(3), 100, true)))
    renderWithProviders(list())
    expect(await screen.findByTestId("job-list")).toBeInTheDocument()
    expect(screen.getByText(/100 total/)).toBeInTheDocument()
    expect(screen.getByText(/Page 1 of 2/)).toBeInTheDocument()
  })

  it("disables Previous on page 0 and enables Next when hasMore=true", async () => {
    listJobs.mockResolvedValue(ok(output(jobs(3), 100, true)))
    const onPageChange = vi.fn()
    renderWithProviders(list({ onPageChange }))
    const prev = await screen.findByRole("button", { name: "Previous" })
    const next = screen.getByRole("button", { name: "Next" })
    expect(prev).toBeDisabled()
    expect(next).not.toBeDisabled()
    fireEvent.click(next)
    expect(onPageChange).toHaveBeenCalledWith(1)
  })

  it("disables Next when hasMore=false", async () => {
    listJobs.mockResolvedValue(ok(output(jobs(3), 3, false)))
    renderWithProviders(list())
    const next = await screen.findByRole("button", { name: "Next" })
    expect(next).toBeDisabled()
  })

  // ── Parent-owned data ────────────────────────────────────────────────
  it("pages over the server total when the parent owns the (filtered) data", async () => {
    const onPageChange = vi.fn()
    renderWithProviders(
      list({ page: 1, onPageChange, jobs: jobs(25), total: 312, hasMore: true, pageSize: 25 }),
    )
    // The footer counts the server total, not the loaded rows.
    expect(await screen.findByText(/Page 2 of 13/)).toBeInTheDocument()
    expect(screen.getByText(/312 total/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole("button", { name: "Next" }))
    expect(onPageChange).toHaveBeenCalledWith(2)
    // The parent's query is the only fetch.
    expect(listJobs).not.toHaveBeenCalled()
  })

  it("offers the page sizes and reports a change", async () => {
    const onPageSizeChange = vi.fn()
    renderWithProviders(list({ jobs: jobs(1), total: 1, pageSize: 50, onPageSizeChange }))
    const select = (await screen.findByTestId("job-list-page-size")) as HTMLSelectElement
    expect(select.value).toBe("50")
    expect(Array.from(select.options).map((o) => o.value)).toEqual(["25", "50", "100", "250"])
    fireEvent.change(select, { target: { value: "250" } })
    expect(onPageSizeChange).toHaveBeenCalledWith(250)
  })

  it("sends its own sort and page size when self-managed", async () => {
    listJobs.mockResolvedValue(ok(output(jobs(1), 1)))
    renderWithProviders(
      list({ page: 2, pageSize: 100, sort: { field: "duration", descending: true } }),
    )
    await screen.findByTestId("job-list")
    expect(listJobs).toHaveBeenCalledWith(
      expect.objectContaining({
        offset: 200,
        limit: 100,
        sort: { field: "duration", descending: true },
      }),
    )
  })

  it("says the filter matched nothing, and shows the parent's loading / error state", async () => {
    const { rerender } = renderWithProviders(
      list({ state: "failed", jobs: [], total: 0, filtered: true }),
    )
    expect(await screen.findByText(/No failed jobs match the filter/)).toBeInTheDocument()

    rerender(list({ state: "failed", jobs: [], loading: true }))
    expect(screen.getByRole("status", { name: "Loading jobs" })).toBeInTheDocument()

    rerender(list({ state: "failed", jobs: [], errorMessage: "Redis went away" }))
    expect(screen.getByRole("alert")).toHaveTextContent(/Redis went away/)
  })
})
