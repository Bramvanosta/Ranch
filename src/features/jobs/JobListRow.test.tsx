/**
 * JobListRow rendering + click-to-drill-down tests.
 *
 * The row now navigates programmatically via `useNavigate` (the
 * old `Link`-inside-`ContextMenuTrigger` pattern intercepted pointer
 * events live; switching avoids that). We stub the router with a
 * mocked `useNavigate` and assert it gets called with the right
 * params on click.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

const copySpy = vi.fn()
vi.mock("@/hooks/useCopyToClipboard", () => ({
  useCopyToClipboard: () => copySpy,
}))
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    retryJob: vi.fn(),
    removeJob: vi.fn(),
    promoteJob: vi.fn(),
  },
}))

const navigateSpy = vi.fn()
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigateSpy,
}))

import type { Job } from "@/lib/tauri/bindings"
import { useJobSelectionStore } from "@/stores/useJobSelectionStore"
import { JobListRow } from "./JobListRow"
import { jobColumns } from "./jobColumns"

function jobFixture(overrides: Partial<Job> = {}): Job {
  return {
    jobRef: { queue: { prefix: "bull", name: "emails" }, id: "42" },
    name: "send-email",
    state: "waiting",
    data: '{"to":"a@b.c"}',
    opts: "{}",
    attemptsMade: 0,
    timestamp: 1_700_000_000_000,
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

const NOW = 1_700_000_300_000 // 5 minutes after the fixture timestamp.

function renderWithProvider(ui: React.ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>)
}

afterEach(() => {
  vi.clearAllMocks()
})

describe("JobListRow", () => {
  it("renders id, name, age, status, and includes them in the aria-label", () => {
    renderWithProvider(<JobListRow job={jobFixture()} now={NOW} connectionId="c1" />)
    const row = screen.getByTestId("job-list-row")
    const ariaLabel = row.getAttribute("aria-label") ?? ""
    expect(ariaLabel).toContain("Job 42")
    expect(ariaLabel).toContain("send-email")
    expect(ariaLabel).toContain("waiting")
    expect(ariaLabel).toContain("5m ago")
  })

  it("clicking the row calls navigate() with the drill-down route params", () => {
    renderWithProvider(<JobListRow job={jobFixture()} now={NOW} connectionId="c1" />)
    fireEvent.click(screen.getByTestId("job-list-row"))
    expect(navigateSpy).toHaveBeenCalledTimes(1)
    expect(navigateSpy).toHaveBeenCalledWith({
      to: "/connections/$connectionId/queues/$queueName/$jobId",
      params: { connectionId: "c1", queueName: "emails", jobId: "42" },
    })
  })

  it("pressing Enter on a focused row also navigates", () => {
    renderWithProvider(<JobListRow job={jobFixture()} now={NOW} connectionId="c1" />)
    fireEvent.keyDown(screen.getByTestId("job-list-row"), { key: "Enter" })
    expect(navigateSpy).toHaveBeenCalledTimes(1)
  })

  it("exposes the drill-down path as data-href for routing tooling", () => {
    renderWithProvider(<JobListRow job={jobFixture()} now={NOW} connectionId="c1" />)
    expect(screen.getByTestId("job-list-row")).toHaveAttribute(
      "data-href",
      "/connections/c1/queues/emails/42",
    )
  })

  // ── The columns of each state ───────────────────────────

  it("the whole id, in its own column, never cut — and no payload glimpse", () => {
    renderWithProvider(
      <JobListRow
        job={jobFixture({
          jobRef: { queue: { prefix: "bull", name: "emails" }, id: "enrich-film-tmdb-603" },
          data: '{"filmId":603,"source":"tmdb"}',
        })}
        now={NOW}
        connectionId="c1"
      />,
    )
    const id = screen.getByTestId("job-list-row-id")
    expect(id).toHaveTextContent("#enrich-film-tmdb-603")
    expect(id.className).toContain("break-all")
    expect(id.className).not.toContain("truncate")
    expect(screen.queryByText(/filmId/)).toBeNull()
  })

  it("failed: the error in full (summarised, wrapping), the tries, when it failed, how long it ran", () => {
    renderWithProvider(
      <JobListRow
        job={jobFixture({
          state: "failed",
          attemptsMade: 3,
          opts: '{"attempts":3}',
          failedReason:
            "\nInvalid `tx.movie.createMany()` invocation:\n\n\nUnique constraint failed",
          processedOn: NOW - 5 * 60_000 - 2_000,
          finishedOn: NOW - 5 * 60_000,
        })}
        now={NOW}
        connectionId="c1"
      />,
    )
    const error = screen.getByTestId("job-list-row-error-cell")
    expect(error).toHaveTextContent("Unique constraint failed — tx.movie.createMany()")
    expect(error.className).toContain("break-words")
    expect(error.className).not.toMatch(/truncate|line-clamp/)
    expect(screen.getByTestId("job-list-row-attempts")).toHaveTextContent("3/3")
    expect(screen.getByTestId("job-list-row-milestone")).toHaveTextContent("5m")
    expect(screen.getByTestId("job-list-row-duration")).toHaveTextContent("2.00 s")
  })

  it("waiting: when it was added, its priority and its delay", () => {
    renderWithProvider(
      <JobListRow job={jobFixture({ opts: '{"priority":3}' })} now={NOW} connectionId="c1" />,
    )
    expect(screen.getByTestId("job-list-row-age")).toHaveTextContent("5m")
    expect(screen.getByText("3")).toBeInTheDocument()
    expect(screen.queryByTestId("job-list-row-error-cell")).toBeNull()
    expect(screen.queryByTestId("job-list-row-duration")).toBeNull()
  })

  it("the list is one state: the row does not repeat it, a screen reader still hears it", () => {
    renderWithProvider(
      <JobListRow
        job={jobFixture({ state: "failed", failedReason: "Connection refused" })}
        now={NOW}
        connectionId="c1"
      />,
    )
    expect(screen.queryByText("failed")).not.toBeInTheDocument()
    expect(screen.getByTestId("job-list-row")).toHaveAttribute(
      "aria-label",
      expect.stringContaining("failed"),
    )
  })

  it("clicking the checkbox does NOT navigate (event isolation)", () => {
    renderWithProvider(<JobListRow job={jobFixture()} now={NOW} connectionId="c1" />)
    fireEvent.click(screen.getByTestId("job-list-row-checkbox"))
    expect(navigateSpy).not.toHaveBeenCalled()
  })

  it("uses tabular monospace for the job ID", () => {
    renderWithProvider(<JobListRow job={jobFixture()} now={NOW} connectionId="c1" />)
    expect(screen.getByTestId("job-list-row-id").className).toMatch(/font-mono.*tabular-nums/)
  })

  it("renders the Queue cell in the all-queues scope only", () => {
    const { rerender } = renderWithProvider(
      <JobListRow job={jobFixture()} now={NOW} connectionId="c1" />,
    )
    expect(screen.queryByTestId("job-list-row-queue-cell")).not.toBeInTheDocument()
    rerender(
      <QueryClientProvider client={new QueryClient()}>
        <JobListRow
          job={jobFixture()}
          now={NOW}
          connectionId="c1"
          columns={jobColumns({ state: "waiting", showQueue: true })}
        />
      </QueryClientProvider>,
    )
    expect(screen.getByTestId("job-list-row-queue-cell")).toHaveTextContent("emails")
  })

  // ── right-click context menu (preserved) ─────────────────

  function openContextMenu() {
    fireEvent.contextMenu(screen.getByTestId("job-list-row"))
  }

  it("right-clicking a waiting job shows Copy ID and Copy Payload (no Copy Stack)", async () => {
    renderWithProvider(<JobListRow job={jobFixture()} now={NOW} connectionId="c1" />)
    openContextMenu()
    expect(await screen.findByTestId("job-list-row-copy-id")).toBeInTheDocument()
    expect(screen.getByTestId("job-list-row-copy-payload")).toBeInTheDocument()
    expect(screen.queryByTestId("job-list-row-copy-stack")).not.toBeInTheDocument()
  })

  it("right-clicking a failed job adds the Copy Stack item", async () => {
    renderWithProvider(
      <JobListRow
        job={jobFixture({
          state: "failed",
          failedReason: "Connection refused",
          stacktrace: ["Error: Connection refused", "    at send"],
        })}
        now={NOW}
        connectionId="c1"
      />,
    )
    openContextMenu()
    expect(await screen.findByTestId("job-list-row-copy-id")).toBeInTheDocument()
    expect(screen.getByTestId("job-list-row-copy-payload")).toBeInTheDocument()
    expect(screen.getByTestId("job-list-row-copy-stack")).toBeInTheDocument()
  })

  // Regression guard: the items used to pass `onSelect` (a Radix prop). Base
  // UI's `Menu.Item` only knows `onClick`, so every action was dead.
  it("clicking Copy ID actually copies the job id", async () => {
    renderWithProvider(<JobListRow job={jobFixture()} now={NOW} connectionId="c1" />)
    openContextMenu()
    fireEvent.click(await screen.findByTestId("job-list-row-copy-id"))
    expect(copySpy).toHaveBeenCalledWith(jobFixture().jobRef.id, "job ID")
  })

  // ── Durations ────────────────────────────────────────────────────────

  it("completed: finishedOn − processedOn, under Took", () => {
    renderWithProvider(
      <JobListRow
        job={jobFixture({ state: "completed", processedOn: NOW - 2_500, finishedOn: NOW - 500 })}
        now={NOW}
        connectionId="c1"
      />,
    )
    expect(screen.getByTestId("job-list-row-duration")).toHaveTextContent("2.00 s")
  })

  it("active: the running time ticks with now", () => {
    renderWithProvider(
      <JobListRow
        job={jobFixture({ state: "active", processedOn: NOW - 1_200 })}
        now={NOW}
        connectionId="c1"
      />,
    )
    expect(screen.getByTestId("job-running-time")).toHaveTextContent("1.20 s")
  })

  // ── Progress (the Active list's column) ─────────────────

  it("an active job that reported no progress shows a dash", () => {
    renderWithProvider(
      <JobListRow job={jobFixture({ state: "active" })} now={NOW} connectionId="c1" />,
    )
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument()
    expect(screen.getByTestId("job-progress-cell")).toHaveTextContent("—")
  })

  it("renders a numeric progress as a bar with its percentage", () => {
    renderWithProvider(
      <JobListRow
        job={jobFixture({ state: "active", progress: "42" })}
        now={NOW}
        connectionId="c1"
      />,
    )
    const bar = screen.getByRole("progressbar", { name: "Job progress" })
    expect(bar).toHaveAttribute("aria-valuenow", "42")
    expect(screen.getByTestId("job-progress-cell")).toHaveTextContent("42%")
  })

  it("renders an object progress without a percentage as text, never as a bar", () => {
    renderWithProvider(
      <JobListRow
        job={jobFixture({ state: "active", progress: '{"step":"upload"}' })}
        now={NOW}
        connectionId="c1"
      />,
    )
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument()
    expect(screen.getByTestId("job-progress-cell")).toHaveTextContent('{"step":"upload"}')
    expect(screen.getByTestId("job-progress-note")).toHaveTextContent("upload")
  })
})

describe("JobListRow — right-click on a selection", () => {
  afterEach(() => useJobSelectionStore.getState().clear())

  it("acts on the whole selection and says so in its first line", async () => {
    const job = jobFixture({ state: "failed" })
    const store = useJobSelectionStore.getState()
    store.add({ jobRef: job.jobRef, state: "failed" })
    store.add({ jobRef: { ...job.jobRef, id: "43" }, state: "failed" })
    store.add({ jobRef: { ...job.jobRef, id: "44" }, state: "failed" })
    renderWithProvider(<JobListRow job={job} now={NOW} connectionId="c1" />)

    fireEvent.contextMenu(screen.getByTestId("job-list-row"))
    const items = await screen.findAllByRole("menuitem")
    expect(items[0]).toHaveTextContent("Retry 3 Jobs")
    fireEvent.click(items[0] as HTMLElement)
    expect(useJobSelectionStore.getState().requestedAction).toBe("retry")
  })

  it("a single row offers its own actions, Retry only when it failed", async () => {
    renderWithProvider(
      <JobListRow job={jobFixture({ state: "delayed" })} now={NOW} connectionId="c1" />,
    )
    fireEvent.contextMenu(screen.getByTestId("job-list-row"))
    const retry = await screen.findByRole("menuitem", { name: "Retry" })
    expect(retry).toHaveAttribute("aria-disabled", "true")
    expect(screen.getByRole("menuitem", { name: "Promote" })).not.toHaveAttribute(
      "aria-disabled",
      "true",
    )
  })
})

describe("JobListRow — clicks", () => {
  afterEach(() => useJobSelectionStore.getState().clear())

  it("a plain click opens the job beside the list when the screen has an inspector", () => {
    const onOpen = vi.fn()
    const job = jobFixture()
    renderWithProvider(<JobListRow job={job} now={NOW} connectionId="c1" onOpen={onOpen} />)
    fireEvent.click(screen.getByTestId("job-list-row"))
    expect(onOpen).toHaveBeenCalledWith(job)
  })

  it("⌘-click adds the row to the selection, ⇧-click extends it", () => {
    const onOpen = vi.fn()
    const onShift = vi.fn()
    renderWithProvider(
      <JobListRow
        job={jobFixture()}
        now={NOW}
        connectionId="c1"
        index={3}
        onOpen={onOpen}
        onShiftClickCheckbox={onShift}
      />,
    )
    const row = screen.getByTestId("job-list-row")
    fireEvent.click(row, { metaKey: true })
    expect(useJobSelectionStore.getState().selection.size).toBe(1)
    fireEvent.click(row, { shiftKey: true })
    expect(onShift).toHaveBeenCalledWith(3)
    expect(onOpen).not.toHaveBeenCalled()
  })
})
