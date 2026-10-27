import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { TooltipProvider } from "@/components/ui/tooltip"
import { pressChord } from "@/test/pressChord"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    retryJob: vi.fn(),
    removeJob: vi.fn(),
    promoteJob: vi.fn(),
  },
}))

import { commands, type Job } from "@/lib/tauri/bindings"

import { JobActions } from "./JobActions"

const mockedRetry = commands.retryJob as unknown as ReturnType<typeof vi.fn>
const mockedRemove = commands.removeJob as unknown as ReturnType<typeof vi.fn>
const mockedPromote = commands.promoteJob as unknown as ReturnType<typeof vi.fn>

function makeJob(state: Job["state"], failedReason: string | null = null): Job {
  return {
    jobRef: { queue: { prefix: "bull", name: "emails" }, id: "42" },
    name: "send-email",
    data: "{}",
    opts: "{}",
    state,
    timestamp: 1700000000000,
    attemptsMade: 1,
    failedReason,
    stacktrace: [],
    processedOn: null,
    finishedOn: null,
    returnValue: null,
    parentKey: null,
    progress: null,
    delay: null,
  }
}

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>
      <TooltipProvider>{children}</TooltipProvider>
    </QueryClientProvider>
  )
}

beforeEach(() => {
  Object.defineProperty(navigator, "platform", { value: "MacIntel", configurable: true })
})

afterEach(() => {
  vi.resetAllMocks()
})

// ─── Full variant ───────────────────────────────────────────────────────

describe("JobActions (full variant)", () => {
  it("offers Retry only on a failed job — and no greyed-out Promote", () => {
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("failed", "kaboom")}
        />
      </Wrap>,
    )
    expect(screen.getByTestId("job-actions-retry")).not.toBeDisabled()
    expect(screen.queryByTestId("job-actions-promote")).not.toBeInTheDocument()
    expect(screen.getByTestId("job-actions-remove")).not.toBeDisabled()
  })

  it("offers Promote only on a delayed job", () => {
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("delayed")}
        />
      </Wrap>,
    )
    expect(screen.queryByTestId("job-actions-retry")).not.toBeInTheDocument()
    expect(screen.getByTestId("job-actions-promote")).not.toBeDisabled()
    expect(screen.getByTestId("job-actions-remove")).not.toBeDisabled()
  })

  it("clicking Retry fires the mutation", async () => {
    mockedRetry.mockResolvedValue({
      status: "ok",
      data: { jobId: "42", newState: "waiting", action: "retry" },
    })
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("failed", "k")}
        />
      </Wrap>,
    )
    await act(async () => {
      fireEvent.click(screen.getByTestId("job-actions-retry"))
    })
    await waitFor(() =>
      expect(mockedRetry).toHaveBeenCalledWith({
        connectionId: "conn-x",
        queueRef: { prefix: "bull", name: "emails" },
        jobId: "42",
      }),
    )
  })

  it("Remove asks first: nothing is deleted until the dialog is confirmed", async () => {
    mockedRemove.mockResolvedValue({
      status: "ok",
      data: { jobId: "42", newState: null, action: "remove" },
    })
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("completed")}
        />
      </Wrap>,
    )
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
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("completed")}
        />
      </Wrap>,
    )
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
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("completed")}
        />
      </Wrap>,
    )
    await act(async () => {
      pressChord("cmd+backspace")
    })
    expect(await screen.findByTestId("job-remove-confirm")).toBeInTheDocument()
    expect(mockedRemove).not.toHaveBeenCalled()
  })

  it("tells the page once the job is gone — and not when the removal failed", async () => {
    const onRemoved = vi.fn()
    mockedRemove.mockResolvedValueOnce({
      status: "error",
      error: { kind: "connectionLocked", message: "locked", docUrl: null },
    })
    mockedRemove.mockResolvedValue({
      status: "ok",
      data: { jobId: "42", newState: null, action: "remove" },
    })
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("completed")}
          onRemoved={onRemoved}
        />
      </Wrap>,
    )
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
    mockedPromote.mockResolvedValue({
      status: "ok",
      data: { jobId: "42", newState: "waiting", action: "promote" },
    })
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("delayed")}
        />
      </Wrap>,
    )
    await act(async () => {
      fireEvent.click(screen.getByTestId("job-actions-promote"))
    })
    await waitFor(() => expect(mockedPromote).toHaveBeenCalled())
  })

  it("⇧⌘R fires Retry when failed", async () => {
    mockedRetry.mockResolvedValue({
      status: "ok",
      data: { jobId: "42", newState: "waiting", action: "retry" },
    })
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("failed", "k")}
        />
      </Wrap>,
    )
    await act(async () => {
      pressChord("cmd+shift+r")
    })
    await waitFor(() => expect(mockedRetry).toHaveBeenCalled())
  })

  it("⇧⌘R is a no-op when state is not failed", async () => {
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("waiting")}
        />
      </Wrap>,
    )
    await act(async () => {
      pressChord("cmd+shift+r")
    })
    // Give the mutation a tick to fire if it were going to.
    await new Promise((r) => setTimeout(r, 10))
    expect(mockedRetry).not.toHaveBeenCalled()
  })

  it("⌥⌘P fires Promote when delayed", async () => {
    mockedPromote.mockResolvedValue({
      status: "ok",
      data: { jobId: "42", newState: "waiting", action: "promote" },
    })
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("delayed")}
        />
      </Wrap>,
    )
    await act(async () => {
      pressChord("alt+cmd+p")
    })
    await waitFor(() => expect(mockedPromote).toHaveBeenCalled())
  })

  it("shows mac keystroke hints on macIntel", () => {
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("failed", "k")}
        />
      </Wrap>,
    )
    expect(screen.getByTestId("job-actions-retry").textContent).toContain("⇧⌘R")
  })

  it("shows ctrl keystroke hints on non-mac", () => {
    Object.defineProperty(navigator, "platform", { value: "Linux x86_64", configurable: true })
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("failed", "k")}
        />
      </Wrap>,
    )
    expect(screen.getByTestId("job-actions-retry").textContent).toContain("Ctrl+Shift+R")
  })
})

// ─── Compact variant ────────────────────────────────────────────────────

describe("JobActions (compact variant)", () => {
  it("renders only Retry icon on failed", () => {
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("failed", "k")}
          compact
        />
      </Wrap>,
    )
    expect(screen.getByTestId("job-actions-retry")).toBeInTheDocument()
    expect(screen.queryByTestId("job-actions-promote")).toBeNull()
    expect(screen.queryByTestId("job-actions-remove")).toBeNull()
  })

  it("renders only Promote icon on delayed", () => {
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("delayed")}
          compact
        />
      </Wrap>,
    )
    expect(screen.getByTestId("job-actions-promote")).toBeInTheDocument()
    expect(screen.queryByTestId("job-actions-retry")).toBeNull()
  })

  it("renders nothing for other states", () => {
    const Wrap = wrap()
    const { container } = render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("waiting")}
          compact
        />
      </Wrap>,
    )
    expect(screen.queryByTestId("job-actions-retry")).toBeNull()
    expect(screen.queryByTestId("job-actions-promote")).toBeNull()
    expect(container.querySelector("[data-testid='job-actions-compact']")).toBeInTheDocument()
  })

  it("does NOT fire ⇧⌘R when compact (shortcuts disabled)", async () => {
    const Wrap = wrap()
    render(
      <Wrap>
        <JobActions
          connectionId="conn-x"
          queueRef={{ prefix: "bull", name: "emails" }}
          job={makeJob("failed", "k")}
          compact
        />
      </Wrap>,
    )
    await act(async () => {
      pressChord("cmd+shift+r")
    })
    await new Promise((r) => setTimeout(r, 10))
    expect(mockedRetry).not.toHaveBeenCalled()
  })
})
