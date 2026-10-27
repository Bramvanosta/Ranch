import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listJobs: vi.fn() },
}))

import { commands, type Job, type ListJobsOutput } from "@/lib/tauri/bindings"

import { JobList } from "./JobList"

const mocked = commands as unknown as { listJobs: ReturnType<typeof vi.fn> }

afterEach(() => {
  vi.resetAllMocks()
})

function wrapper() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

function makeJob(id: string, overrides: Partial<Job> = {}): Job {
  return {
    jobRef: { queue: { prefix: "bull", name: "emails" }, id },
    name: `job-${id}`,
    state: "waiting",
    data: "{}",
    opts: "{}",
    attemptsMade: 0,
    timestamp: Date.now() - 60_000,
    processedOn: null,
    finishedOn: null,
    delay: null,
    failedReason: null,
    stacktrace: [],
    returnValue: null,
    parentKey: null,
    progress: null,
    ...overrides,
  }
}

function output(jobs: Job[], total = jobs.length, hasMore = false): ListJobsOutput {
  return { jobs, total, state: "waiting", hasMore, scanned: null, scanCapped: false }
}

const queueRef = { prefix: "bull", name: "emails" }

describe("JobList", () => {
  it("shows the empty state when total=0", async () => {
    mocked.listJobs.mockResolvedValue({ status: "ok", data: output([], 0) })
    const Wrapper = wrapper()
    render(
      <Wrapper>
        <JobList
          connectionId="c1"
          queueRef={queueRef}
          state="waiting"
          page={0}
          onPageChange={vi.fn()}
        />
      </Wrapper>,
    )
    expect(await screen.findByText(/No waiting jobs in this queue/)).toBeInTheDocument()
  })

  it("renders an error block when the query fails", async () => {
    mocked.listJobs.mockResolvedValue({
      status: "error",
      error: { kind: "schemaNotRecognized", message: "Schema 7.x not supported", docUrl: null },
    })
    const Wrapper = wrapper()
    render(
      <Wrapper>
        <JobList
          connectionId="c1"
          queueRef={queueRef}
          state="failed"
          page={0}
          onPageChange={vi.fn()}
        />
      </Wrapper>,
    )
    expect(await screen.findByRole("alert")).toHaveTextContent(/Could not list jobs/)
    expect(screen.getByRole("alert")).toHaveTextContent(/Schema 7.x not supported/)
  })

  it("renders rows + pagination footer when populated", async () => {
    const jobs = Array.from({ length: 3 }, (_, i) => makeJob(String(i + 1)))
    mocked.listJobs.mockResolvedValue({ status: "ok", data: output(jobs, 100, true) })
    const Wrapper = wrapper()
    render(
      <Wrapper>
        <JobList
          connectionId="c1"
          queueRef={queueRef}
          state="waiting"
          page={0}
          onPageChange={vi.fn()}
        />
      </Wrapper>,
    )
    expect(await screen.findByTestId("job-list")).toBeInTheDocument()
    expect(screen.getByText(/100 total/)).toBeInTheDocument()
    expect(screen.getByText(/Page 1 of 2/)).toBeInTheDocument()
  })

  it("disables Previous on page 0 and enables Next when hasMore=true", async () => {
    const jobs = Array.from({ length: 3 }, (_, i) => makeJob(String(i + 1)))
    mocked.listJobs.mockResolvedValue({ status: "ok", data: output(jobs, 100, true) })
    const onPageChange = vi.fn()
    const Wrapper = wrapper()
    render(
      <Wrapper>
        <JobList
          connectionId="c1"
          queueRef={queueRef}
          state="waiting"
          page={0}
          onPageChange={onPageChange}
        />
      </Wrapper>,
    )
    const prev = await screen.findByRole("button", { name: "Previous" })
    const next = screen.getByRole("button", { name: "Next" })
    expect(prev).toBeDisabled()
    expect(next).not.toBeDisabled()
    fireEvent.click(next)
    expect(onPageChange).toHaveBeenCalledWith(1)
  })

  it("disables Next when hasMore=false", async () => {
    const jobs = Array.from({ length: 3 }, (_, i) => makeJob(String(i + 1)))
    mocked.listJobs.mockResolvedValue({ status: "ok", data: output(jobs, 3, false) })
    const Wrapper = wrapper()
    render(
      <Wrapper>
        <JobList
          connectionId="c1"
          queueRef={queueRef}
          state="waiting"
          page={0}
          onPageChange={vi.fn()}
        />
      </Wrapper>,
    )
    const next = await screen.findByRole("button", { name: "Next" })
    expect(next).toBeDisabled()
  })
  // ── Parent-owned data ────────────────────────────────────────────────
  it("pages over the server total when the parent owns the (filtered) data", async () => {
    const jobs = Array.from({ length: 25 }, (_, i) => makeJob(String(i + 1)))
    const onPageChange = vi.fn()
    const Wrapper = wrapper()
    render(
      <Wrapper>
        <JobList
          connectionId="c1"
          queueRef={queueRef}
          state="waiting"
          page={1}
          onPageChange={onPageChange}
          jobs={jobs}
          total={312}
          hasMore
          pageSize={25}
        />
      </Wrapper>,
    )
    // Was "Page 2 of 1 (25 total)": the footer used the loaded rows as the total.
    expect(await screen.findByText(/Page 2 of 13/)).toBeInTheDocument()
    expect(screen.getByText(/312 total/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole("button", { name: "Next" }))
    expect(onPageChange).toHaveBeenCalledWith(2)
    // The parent's query is the only fetch.
    expect(mocked.listJobs).not.toHaveBeenCalled()
  })

  it("offers the page sizes and reports a change", async () => {
    const onPageSizeChange = vi.fn()
    const Wrapper = wrapper()
    render(
      <Wrapper>
        <JobList
          connectionId="c1"
          queueRef={queueRef}
          state="waiting"
          page={0}
          onPageChange={vi.fn()}
          jobs={[makeJob("1")]}
          total={1}
          pageSize={50}
          onPageSizeChange={onPageSizeChange}
        />
      </Wrapper>,
    )
    const select = (await screen.findByTestId("job-list-page-size")) as HTMLSelectElement
    expect(select.value).toBe("50")
    expect(Array.from(select.options).map((o) => o.value)).toEqual(["25", "50", "100", "250"])
    fireEvent.change(select, { target: { value: "250" } })
    expect(onPageSizeChange).toHaveBeenCalledWith(250)
  })

  it("sends its own sort and page size when self-managed", async () => {
    mocked.listJobs.mockResolvedValue({ status: "ok", data: output([makeJob("1")], 1) })
    const Wrapper = wrapper()
    render(
      <Wrapper>
        <JobList
          connectionId="c1"
          queueRef={queueRef}
          state="waiting"
          page={2}
          onPageChange={vi.fn()}
          pageSize={100}
          sort={{ field: "duration", descending: true }}
        />
      </Wrapper>,
    )
    await screen.findByTestId("job-list")
    expect(mocked.listJobs).toHaveBeenCalledWith(
      expect.objectContaining({
        offset: 200,
        limit: 100,
        sort: { field: "duration", descending: true },
      }),
    )
  })

  it("says the filter matched nothing, and shows the parent's loading / error state", async () => {
    const Wrapper = wrapper()
    const { rerender } = render(
      <Wrapper>
        <JobList
          connectionId="c1"
          queueRef={queueRef}
          state="failed"
          page={0}
          onPageChange={vi.fn()}
          jobs={[]}
          total={0}
          filtered
        />
      </Wrapper>,
    )
    expect(await screen.findByText(/No failed jobs match the filter/)).toBeInTheDocument()

    rerender(
      <Wrapper>
        <JobList
          connectionId="c1"
          queueRef={queueRef}
          state="failed"
          page={0}
          onPageChange={vi.fn()}
          jobs={[]}
          loading
        />
      </Wrapper>,
    )
    expect(screen.getByRole("status", { name: "Loading jobs" })).toBeInTheDocument()

    rerender(
      <Wrapper>
        <JobList
          connectionId="c1"
          queueRef={queueRef}
          state="failed"
          page={0}
          onPageChange={vi.fn()}
          jobs={[]}
          errorMessage="Redis went away"
        />
      </Wrapper>,
    )
    expect(screen.getByRole("alert")).toHaveTextContent(/Redis went away/)
  })
})
