import { act, fireEvent, screen, waitFor } from "@testing-library/react"
import type { ComponentProps } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    retryJob: vi.fn(),
    removeJob: vi.fn(),
    promoteJob: vi.fn(),
  },
}))

import { commands, type Job } from "@/lib/tauri/bindings"
import { aJob, aQueueId } from "@/test/factories"
import { pressChord } from "@/test/pressChord"
import { renderWithProviders } from "@/test/renderWithProviders"
import { err, ok } from "@/test/results"

import { JobActions } from "./JobActions"

const mockedRetry = vi.mocked(commands.retryJob)
const mockedRemove = vi.mocked(commands.removeJob)
const mockedPromote = vi.mocked(commands.promoteJob)

function job(state: Job["state"], failedReason: string | null = null): Job {
  return aJob({ id: "42", state, failedReason, attemptsMade: 1, timestamp: 1_700_000_000_000 })
}

function actions(job: Job, props: Partial<ComponentProps<typeof JobActions>> = {}) {
  return <JobActions connectionId="conn-x" queueId={aQueueId()} job={job} {...props} />
}

beforeEach(() => {
  Object.defineProperty(navigator, "platform", { value: "MacIntel", configurable: true })
})

afterEach(() => {
  vi.resetAllMocks()
})

// ─── Full variant ───────────────────────────────────────────────────────

describe("JobActions (full variant)", () => {
  it("offers Retry only on a failed job, and no greyed-out Promote", () => {
    renderWithProviders(actions(job("failed", "kaboom")))
    expect(screen.getByTestId("job-actions-retry")).not.toBeDisabled()
    expect(screen.queryByTestId("job-actions-promote")).not.toBeInTheDocument()
    expect(screen.getByTestId("job-actions-remove")).not.toBeDisabled()
  })

  it("offers Promote only on a delayed job", () => {
    renderWithProviders(actions(job("delayed")))
    expect(screen.queryByTestId("job-actions-retry")).not.toBeInTheDocument()
    expect(screen.getByTestId("job-actions-promote")).not.toBeDisabled()
    expect(screen.getByTestId("job-actions-remove")).not.toBeDisabled()
  })

  it("clicking Retry fires the mutation", async () => {
    mockedRetry.mockResolvedValue(ok({ jobId: "42", newState: "waiting", action: "retry" }))
    renderWithProviders(actions(job("failed", "k")))
    await act(async () => {
      fireEvent.click(screen.getByTestId("job-actions-retry"))
    })
    await waitFor(() =>
      expect(mockedRetry).toHaveBeenCalledWith({
        connectionId: "conn-x",
        queueId: "bull:emails",
        jobId: "42",
      }),
    )
  })

  it("Remove asks first: nothing is deleted until the dialog is confirmed", async () => {
    mockedRemove.mockResolvedValue(ok({ jobId: "42", newState: null, action: "remove" }))
    renderWithProviders(actions(job("completed")))
    expect(screen.queryByTestId("job-remove-confirm")).not.toBeInTheDocument()
    await act(async () => {
      fireEvent.click(screen.getByTestId("job-actions-remove"))
    })
    const dialog = await screen.findByTestId("job-remove-confirm")
    expect(dialog).toHaveTextContent("#42")
    expect(dialog).toHaveTextContent("Every other job of emails")
    expect(dialog).toHaveTextContent("cannot be undone")
    expect(mockedRemove).not.toHaveBeenCalled()

    await act(async () => {
      fireEvent.click(screen.getByTestId("job-remove-confirm-action"))
    })
    await waitFor(() => expect(mockedRemove).toHaveBeenCalledTimes(1))
  })

  it("Cancel in the dialog deletes nothing", async () => {
    renderWithProviders(actions(job("completed")))
    await act(async () => {
      fireEvent.click(screen.getByTestId("job-actions-remove"))
    })
    await screen.findByTestId("job-remove-confirm")
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Cancel" }))
    })
    await waitFor(() => expect(screen.queryByTestId("job-remove-confirm")).not.toBeInTheDocument())
    expect(mockedRemove).not.toHaveBeenCalled()
  })

  it("⌘⌫ asks too", async () => {
    renderWithProviders(actions(job("completed")))
    await act(async () => {
      pressChord("cmd+backspace")
    })
    expect(await screen.findByTestId("job-remove-confirm")).toBeInTheDocument()
    expect(mockedRemove).not.toHaveBeenCalled()
  })

  it("tells the page once the job is gone, and not when the removal failed", async () => {
    const onRemoved = vi.fn()
    mockedRemove.mockResolvedValueOnce(err("connectionLocked", "locked"))
    mockedRemove.mockResolvedValue(ok({ jobId: "42", newState: null, action: "remove" }))
    renderWithProviders(actions(job("completed"), { onRemoved }))
    const removeAndConfirm = async () => {
      await act(async () => {
        fireEvent.click(screen.getByTestId("job-actions-remove"))
      })
      await act(async () => {
        fireEvent.click(await screen.findByTestId("job-remove-confirm-action"))
      })
    }
    await removeAndConfirm()
    await waitFor(() => expect(mockedRemove).toHaveBeenCalledTimes(1))
    expect(onRemoved).not.toHaveBeenCalled()
    // The refused removal closed the dialog (the toast says why).
    await waitFor(() => expect(screen.queryByTestId("job-remove-confirm")).not.toBeInTheDocument())

    await removeAndConfirm()
    await waitFor(() => expect(onRemoved).toHaveBeenCalledTimes(1))
  })

  it("clicking Promote fires the mutation when delayed", async () => {
    mockedPromote.mockResolvedValue(ok({ jobId: "42", newState: "waiting", action: "promote" }))
    renderWithProviders(actions(job("delayed")))
    await act(async () => {
      fireEvent.click(screen.getByTestId("job-actions-promote"))
    })
    await waitFor(() => expect(mockedPromote).toHaveBeenCalled())
  })

  it("⇧⌘R fires Retry when failed", async () => {
    mockedRetry.mockResolvedValue(ok({ jobId: "42", newState: "waiting", action: "retry" }))
    renderWithProviders(actions(job("failed", "k")))
    await act(async () => {
      pressChord("cmd+shift+r")
    })
    await waitFor(() => expect(mockedRetry).toHaveBeenCalled())
  })

  it("⇧⌘R is a no-op when state is not failed", async () => {
    renderWithProviders(actions(job("waiting")))
    await act(async () => {
      pressChord("cmd+shift+r")
    })
    // Give the mutation a tick to fire if it were going to.
    await new Promise((r) => setTimeout(r, 10))
    expect(mockedRetry).not.toHaveBeenCalled()
  })

  it("⌥⌘P fires Promote when delayed", async () => {
    mockedPromote.mockResolvedValue(ok({ jobId: "42", newState: "waiting", action: "promote" }))
    renderWithProviders(actions(job("delayed")))
    await act(async () => {
      pressChord("alt+cmd+p")
    })
    await waitFor(() => expect(mockedPromote).toHaveBeenCalled())
  })

  it("shows mac keystroke hints on macIntel", () => {
    renderWithProviders(actions(job("failed", "k")))
    expect(screen.getByTestId("job-actions-retry").textContent).toContain("⇧⌘R")
  })

  it("shows ctrl keystroke hints on non-mac", () => {
    Object.defineProperty(navigator, "platform", { value: "Linux x86_64", configurable: true })
    renderWithProviders(actions(job("failed", "k")))
    expect(screen.getByTestId("job-actions-retry").textContent).toContain("Ctrl+Shift+R")
  })
})

// ─── Compact variant ────────────────────────────────────────────────────

describe("JobActions (compact variant)", () => {
  it("renders only Retry icon on failed", () => {
    renderWithProviders(actions(job("failed", "k"), { compact: true }))
    expect(screen.getByTestId("job-actions-retry")).toBeInTheDocument()
    expect(screen.queryByTestId("job-actions-promote")).toBeNull()
    expect(screen.queryByTestId("job-actions-remove")).toBeNull()
  })

  it("renders only Promote icon on delayed", () => {
    renderWithProviders(actions(job("delayed"), { compact: true }))
    expect(screen.getByTestId("job-actions-promote")).toBeInTheDocument()
    expect(screen.queryByTestId("job-actions-retry")).toBeNull()
  })

  it("renders nothing for other states", () => {
    const { container } = renderWithProviders(actions(job("waiting"), { compact: true }))
    expect(screen.queryByTestId("job-actions-retry")).toBeNull()
    expect(screen.queryByTestId("job-actions-promote")).toBeNull()
    expect(container.querySelector("[data-testid='job-actions-compact']")).toBeInTheDocument()
  })

  it("does NOT fire ⇧⌘R when compact (shortcuts disabled)", async () => {
    renderWithProviders(actions(job("failed", "k"), { compact: true }))
    await act(async () => {
      pressChord("cmd+shift+r")
    })
    await new Promise((r) => setTimeout(r, 10))
    expect(mockedRetry).not.toHaveBeenCalled()
  })
})
