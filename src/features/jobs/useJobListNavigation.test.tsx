/**
 * `useJobListNavigation` keyboard bindings.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, waitFor } from "@testing-library/react"
import { type ReactNode, useEffect } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
}))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    retryJob: vi.fn(),
    removeJob: vi.fn(),
    promoteJob: vi.fn(),
  },
}))

import { commands, type Job } from "@/lib/tauri/bindings"
import { useJobSelectionStore } from "@/stores/useJobSelectionStore"
import { pressChord } from "@/test/pressChord"

import { useJobListNavigation } from "./useJobListNavigation"

const mocked = commands as unknown as {
  retryJob: ReturnType<typeof vi.fn>
  removeJob: ReturnType<typeof vi.fn>
  promoteJob: ReturnType<typeof vi.fn>
}

function makeJob(id: string): Job {
  return {
    jobRef: { queue: { prefix: "bull", name: "emails" }, id },
    name: "send-email",
    state: "failed",
    data: null,
    opts: null,
    timestamp: Date.now(),
    processedOn: null,
    finishedOn: null,
    attemptsMade: 0,
    delay: 0,
    failedReason: null,
    stacktrace: null,
    returnvalue: null,
    parent: null,
    priority: null,
  } as unknown as Job
}

function wrap(child: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return <QueryClientProvider client={client}>{child}</QueryClientProvider>
}

function TestHarness({
  jobs,
  connectionId,
  capture,
}: {
  jobs: Job[]
  connectionId: string
  capture: (focusedIndex: number) => void
}) {
  const { focusedIndex } = useJobListNavigation({ jobs, connectionId })
  // Surface the current focused index via the capture callback so
  // tests can assert without DOM scraping.
  useEffect(() => {
    capture(focusedIndex)
  })
  return <div data-testid="harness">focused={focusedIndex}</div>
}

afterEach(() => {
  vi.resetAllMocks()
  act(() => useJobSelectionStore.getState().clear())
})

describe("useJobListNavigation", () => {
  it("j and ↓ move the focused index forward", () => {
    const jobs = [makeJob("a"), makeJob("b"), makeJob("c")]
    let observed = -1
    const { getByTestId } = render(
      wrap(<TestHarness jobs={jobs} connectionId="c1" capture={(i) => (observed = i)} />),
    )
    // Force the harness to render before we assert the listener exists.
    expect(getByTestId("harness")).toBeInTheDocument()
    expect(observed).toBe(0)
    // The first step marks the first row — it does not skip it.
    act(() => {
      fireEvent.keyDown(window, { key: "j" })
    })
    expect(observed).toBe(0)
    act(() => {
      fireEvent.keyDown(window, { key: "j" })
    })
    expect(observed).toBe(1)
    act(() => {
      fireEvent.keyDown(window, { key: "ArrowDown" })
    })
    expect(observed).toBe(2)
    // Clamp at the end.
    act(() => {
      fireEvent.keyDown(window, { key: "j" })
    })
    expect(observed).toBe(2)
  })

  it("k and ↑ move backward; PageUp/Down jump 10", () => {
    const jobs = Array.from({ length: 30 }, (_, i) => makeJob(`j${i}`))
    let observed = 0
    render(wrap(<TestHarness jobs={jobs} connectionId="c1" capture={(i) => (observed = i)} />))
    act(() => {
      fireEvent.keyDown(window, { key: "PageDown" })
    })
    expect(observed).toBe(10)
    act(() => {
      fireEvent.keyDown(window, { key: "k" })
    })
    expect(observed).toBe(9)
  })

  it("⇧⌘R fires retryJob with the focused job", async () => {
    mocked.retryJob.mockResolvedValue({
      status: "ok",
      data: { jobId: "a", newState: "waiting", action: "retry" },
    })
    const jobs = [makeJob("a"), makeJob("b")]
    render(wrap(<TestHarness jobs={jobs} connectionId="c1" capture={() => {}} />))
    // Fire BOTH meta + ctrl so the test passes regardless of the
    // jsdom-reported platform. Real users press one or the other.
    // Before the keyboard has marked a row, a chord acts on nothing.
    act(() => {
      pressChord("cmd+shift+r")
    })
    expect(mocked.retryJob).not.toHaveBeenCalled()

    act(() => {
      fireEvent.keyDown(window, { key: "j" })
    })
    act(() => {
      pressChord("cmd+shift+r")
    })
    await waitFor(() => {
      expect(mocked.retryJob).toHaveBeenCalledTimes(1)
    })
    expect(mocked.retryJob.mock.calls[0]?.[0]).toMatchObject({ jobId: "a" })
  })

  it("⌘⌫ deletes nothing: it selects the focused job and asks the bulk bar to confirm", () => {
    const jobs = [makeJob("a"), makeJob("b")]
    render(wrap(<TestHarness jobs={jobs} connectionId="c1" capture={() => {}} />))
    act(() => {
      fireEvent.keyDown(window, { key: "j" })
    })
    act(() => {
      fireEvent.keyDown(window, { key: "j" })
    })
    act(() => {
      pressChord("cmd+backspace")
    })
    expect(mocked.removeJob).not.toHaveBeenCalled()
    const store = useJobSelectionStore.getState()
    expect([...store.selection.values()].map((e) => e.jobRef.id)).toEqual(["b"])
    expect(store.requestedAction).toBe("remove")
  })

  it("with jobs selected, the action chords belong to the bulk bar: the focused row is left alone", () => {
    const jobs = [makeJob("a"), makeJob("b")]
    act(() =>
      useJobSelectionStore
        .getState()
        .add({ jobRef: jobs[1]?.jobRef as Job["jobRef"], state: "failed" }),
    )
    render(wrap(<TestHarness jobs={jobs} connectionId="c1" capture={() => {}} />))
    act(() => {
      pressChord("cmd+shift+r")
      pressChord("cmd+backspace")
    })
    expect(mocked.retryJob).not.toHaveBeenCalled()
    expect(mocked.removeJob).not.toHaveBeenCalled()
    expect(useJobSelectionStore.getState().requestedAction).toBeNull()
  })

  it("input-field guard: typing in an input does not move focus", () => {
    const jobs = [makeJob("a"), makeJob("b"), makeJob("c")]
    let observed = 0
    const Comp = () => (
      <>
        <input data-testid="filter" />
        <TestHarness jobs={jobs} connectionId="c1" capture={(i) => (observed = i)} />
      </>
    )
    render(wrap(<Comp />))
    const input = document.querySelector('[data-testid="filter"]') as HTMLInputElement
    input.focus()
    act(() => {
      fireEvent.keyDown(input, { key: "j" })
    })
    expect(observed).toBe(0)
  })
})
