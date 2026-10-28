/**
 * `useJobListNavigation` keyboard bindings.
 */
import { act, fireEvent, waitFor } from "@testing-library/react"
import { useEffect } from "react"
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
import { aJob } from "@/test/factories"
import { pressChord } from "@/test/pressChord"
import { renderWithProviders } from "@/test/renderWithProviders"
import { ok } from "@/test/results"

import { useJobListNavigation } from "./useJobListNavigation"

const retryJob = vi.mocked(commands.retryJob)
const removeJob = vi.mocked(commands.removeJob)

const failedJob = (id: string) => aJob({ id, state: "failed" })

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
    const jobs = [failedJob("a"), failedJob("b"), failedJob("c")]
    let observed = -1
    const { getByTestId } = renderWithProviders(
      <TestHarness jobs={jobs} connectionId="c1" capture={(i) => (observed = i)} />,
    )
    // Force the harness to render before we assert the listener exists.
    expect(getByTestId("harness")).toBeInTheDocument()
    expect(observed).toBe(0)
    // The first step marks the first row; it does not skip it.
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
    const jobs = Array.from({ length: 30 }, (_, i) => failedJob(`j${i}`))
    let observed = 0
    renderWithProviders(
      <TestHarness jobs={jobs} connectionId="c1" capture={(i) => (observed = i)} />,
    )
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
    retryJob.mockResolvedValue(ok({ jobId: "a", newState: "waiting", action: "retry" }))
    const jobs = [failedJob("a"), failedJob("b")]
    renderWithProviders(<TestHarness jobs={jobs} connectionId="c1" capture={() => {}} />)
    // Fire BOTH meta + ctrl so the test passes regardless of the
    // jsdom-reported platform. Real users press one or the other.
    // Before the keyboard has marked a row, a chord acts on nothing.
    act(() => {
      pressChord("cmd+shift+r")
    })
    expect(retryJob).not.toHaveBeenCalled()

    act(() => {
      fireEvent.keyDown(window, { key: "j" })
    })
    act(() => {
      pressChord("cmd+shift+r")
    })
    await waitFor(() => {
      expect(retryJob).toHaveBeenCalledTimes(1)
    })
    expect(retryJob.mock.calls[0]?.[0]).toMatchObject({ jobId: "a" })
  })

  it("⌘⌫ deletes nothing: it selects the focused job and asks the bulk bar to confirm", () => {
    const jobs = [failedJob("a"), failedJob("b")]
    renderWithProviders(<TestHarness jobs={jobs} connectionId="c1" capture={() => {}} />)
    act(() => {
      fireEvent.keyDown(window, { key: "j" })
    })
    act(() => {
      fireEvent.keyDown(window, { key: "j" })
    })
    act(() => {
      pressChord("cmd+backspace")
    })
    expect(removeJob).not.toHaveBeenCalled()
    const store = useJobSelectionStore.getState()
    expect([...store.selection.values()].map((e) => e.jobRef.id)).toEqual(["b"])
    expect(store.requestedAction).toBe("remove")
  })

  it("with jobs selected, the action chords belong to the bulk bar: the focused row is left alone", () => {
    const jobs = [failedJob("a"), failedJob("b")]
    act(() =>
      useJobSelectionStore
        .getState()
        .add({ jobRef: jobs[1]?.jobRef as Job["jobRef"], state: "failed" }),
    )
    renderWithProviders(<TestHarness jobs={jobs} connectionId="c1" capture={() => {}} />)
    act(() => {
      pressChord("cmd+shift+r")
      pressChord("cmd+backspace")
    })
    expect(retryJob).not.toHaveBeenCalled()
    expect(removeJob).not.toHaveBeenCalled()
    expect(useJobSelectionStore.getState().requestedAction).toBeNull()
  })

  it("input-field guard: typing in an input does not move focus", () => {
    const jobs = [failedJob("a"), failedJob("b"), failedJob("c")]
    let observed = 0
    const Comp = () => (
      <>
        <input data-testid="filter" />
        <TestHarness jobs={jobs} connectionId="c1" capture={(i) => (observed = i)} />
      </>
    )
    renderWithProviders(<Comp />)
    const input = document.querySelector('[data-testid="filter"]') as HTMLInputElement
    input.focus()
    act(() => {
      fireEvent.keyDown(input, { key: "j" })
    })
    expect(observed).toBe(0)
  })
})
