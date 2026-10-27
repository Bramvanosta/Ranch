import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useBulkProgressStore } from "@/stores/useBulkProgressStore"
import { useJobSelectionStore } from "@/stores/useJobSelectionStore"
import { pressChord } from "@/test/pressChord"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    bulkRetryJobs: vi.fn(),
    bulkRemoveJobs: vi.fn(),
    bulkPromoteJobs: vi.fn(),
    listConnections: vi.fn(),
    getConnectionLockState: vi.fn(),
    unlockConnection: vi.fn(),
    lockConnection: vi.fn(),
  },
}))

import { type Connection, commands } from "@/lib/tauri/bindings"

import { BulkActionBar } from "./BulkActionBar"

const mockedRetry = commands.bulkRetryJobs as unknown as ReturnType<typeof vi.fn>
const mockedRemove = commands.bulkRemoveJobs as unknown as ReturnType<typeof vi.fn>
const mockedListConns = commands.listConnections as unknown as ReturnType<typeof vi.fn>
const mockedLockState = commands.getConnectionLockState as unknown as ReturnType<typeof vi.fn>
const mockedUnlock = commands.unlockConnection as unknown as ReturnType<typeof vi.fn>
const mockedLock = commands.lockConnection as unknown as ReturnType<typeof vi.fn>

const queueRef = { prefix: "bull", name: "emails" }

function makeConn(id: string, envColor = "env-local"): Connection {
  return {
    id,
    environmentId: "env-1",
    groupId: "g-1",
    name: `Connection ${id}`,
    host: "127.0.0.1",
    port: 6379,
    db: 0,
    username: null,
    tls: false,
    tlsCustomCa: null,
    sshTunnel: null,
    envColor,
    envName: "Local",
    envLabel: "Local",
    groupName: "Default",
    createdAt: "2026-05-11T12:00:00Z",
    mcpReadonly: null,
    sidebarCollapsed: false,
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
  useJobSelectionStore.getState().clear()
  // Progress store refactored to per-queue maps.
  useBulkProgressStore.getState().finish()
  mockedListConns.mockResolvedValue({ status: "ok", data: [makeConn("conn-x")] })
  mockedLockState.mockResolvedValue({ status: "ok", data: "unlocked" })
})

afterEach(() => {
  vi.resetAllMocks()
})

function seedSelection(jobIds: string[], queueName = "emails") {
  const store = useJobSelectionStore.getState()
  for (const id of jobIds) {
    store.add({
      jobRef: { queue: { prefix: "bull", name: queueName }, id },
      state: "failed",
    })
  }
}

describe("BulkActionBar — selecting state", () => {
  it("hides itself when selection is empty", () => {
    const Wrap = wrap()
    const { container } = render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    expect(container.querySelector("[data-testid='bulk-action-bar']")).toBeNull()
  })

  it("renders the count when jobs are selected", () => {
    seedSelection(["1", "2", "3"])
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    const bar = screen.getByTestId("bulk-action-bar")
    expect(bar).toHaveAttribute("data-state", "selecting")
    expect(screen.getByTestId("bulk-action-bar-count").textContent).toMatch(/3.*1.*queue/)
  })

  it("enables action buttons when selection spans multiple queues (K > 1)", () => {
    const store = useJobSelectionStore.getState()
    store.add({
      jobRef: { queue: { prefix: "bull", name: "emails" }, id: "1" },
      state: "failed",
    })
    store.add({
      jobRef: { queue: { prefix: "bull", name: "reports" }, id: "1" },
      state: "failed",
    })
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    // K>1 goes through the frontend orchestrator, so the action
    // buttons are not disabled.
    expect(screen.getByTestId("bulk-action-retry")).not.toBeDisabled()
    expect(screen.getByTestId("bulk-action-remove")).not.toBeDisabled()
    expect(screen.getByTestId("bulk-action-promote")).not.toBeDisabled()
  })

  it("clicking Cancel clears the selection", async () => {
    seedSelection(["1", "2"])
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-bar-cancel"))
    })
    expect(useJobSelectionStore.getState().selectedCount()).toBe(0)
  })
})

describe("BulkActionBar — preview state", () => {
  it("clicking Retry transitions to preview with the right banner text", async () => {
    seedSelection(["1", "2", "3"])
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-retry"))
    })
    expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "preview")
    expect(screen.getByTestId("bulk-action-bar-preview").textContent).toMatch(
      /Retry 3 jobs.*waiting/,
    )
  })

  it("Remove preview includes the irreversible warning when count > 1", async () => {
    seedSelection(["1", "2"])
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-remove"))
    })
    expect(screen.getByTestId("bulk-action-bar-preview").textContent).toMatch(/irreversible/i)
  })

  it("an action requested from the list (cmd-Backspace on a row) opens the confirmation step", async () => {
    seedSelection(["7"])
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    await act(async () => {
      useJobSelectionStore.getState().requestAction("remove")
    })
    expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "preview")
    // One job: singular — and irreversible all the same.
    const preview = screen.getByTestId("bulk-action-bar-preview").textContent ?? ""
    expect(preview).toMatch(/Remove 1 job across 1 queue/)
    expect(preview).toMatch(/irreversible/i)
    expect(useJobSelectionStore.getState().requestedAction).toBeNull()
  })

  it("Escape from preview returns to selecting", async () => {
    seedSelection(["1"])
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-retry"))
    })
    expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "preview")
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }))
    })
    expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "selecting")
  })
})

describe("BulkActionBar — executing + complete states", () => {
  it("Enter on preview transitions to executing then complete on resolve", async () => {
    seedSelection(["1", "2"])
    mockedRetry.mockResolvedValue({
      status: "ok",
      data: {
        action: "retry",
        total: 2,
        succeeded: 2,
        failed: [],
        cancelled: false,
      },
    })
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-retry"))
    })
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }))
    })
    await waitFor(() =>
      expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "complete"),
    )
    expect(screen.getByTestId("bulk-action-bar-complete").textContent).toMatch(
      /Retried 2 jobs successfully/,
    )
  })

  it("connectionLocked error synthesizes a complete summary with the unlock hint", async () => {
    seedSelection(["1", "2"])
    mockedRetry.mockResolvedValue({
      status: "error",
      error: {
        kind: "connectionLocked",
        message: "Connection prod is locked: Press cmd-shift-U to unlock for this session.",
        docUrl: null,
      },
    })
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-retry"))
    })
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }))
    })
    await waitFor(() =>
      expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "complete"),
    )
    expect(screen.getByTestId("bulk-action-bar-complete").textContent).toMatch(/cmd-shift-U/)
  })

  it("partial failure renders the failures list in the summary", async () => {
    seedSelection(["1", "2", "3"])
    mockedRemove.mockResolvedValue({
      status: "ok",
      data: {
        action: "remove",
        total: 3,
        succeeded: 2,
        failed: [{ jobId: "3", reason: "not found" }],
        cancelled: false,
      },
    })
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-remove"))
    })
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-bar-execute"))
    })
    await waitFor(() =>
      expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "complete"),
    )
    expect(screen.getByTestId("bulk-action-bar-complete").textContent).toMatch(
      /Removed 2 of 3.*1 failed/,
    )
    expect(screen.getByTestId("bulk-action-bar-complete").textContent).toMatch(/#3/)
  })
})

describe("BulkActionBar — keyboard shortcuts", () => {
  it("⇧⌘R from selecting transitions to retry preview", async () => {
    seedSelection(["1"])
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    await act(async () => {
      pressChord("cmd+shift+r")
    })
    expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "preview")
  })

  it("⇧⌘R opens preview when selection is cross-queue (K > 1)", async () => {
    const store = useJobSelectionStore.getState()
    store.add({
      jobRef: { queue: { prefix: "bull", name: "emails" }, id: "1" },
      state: "failed",
    })
    store.add({
      jobRef: { queue: { prefix: "bull", name: "reports" }, id: "1" },
      state: "failed",
    })
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    await act(async () => {
      pressChord("cmd+shift+r")
    })
    // K>1 cmd-R now transitions to preview (cross-queue
    // orchestrator is active).
    expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "preview")
  })
})

// ──────────────────────────────────────────────────────────────────────────
// cross-queue (K>1) BulkActionBar paths
// ──────────────────────────────────────────────────────────────────────────

function seedCrossQueueSelection() {
  const store = useJobSelectionStore.getState()
  store.add({
    jobRef: { queue: { prefix: "bull", name: "emails" }, id: "1" },
    state: "failed",
  })
  store.add({
    jobRef: { queue: { prefix: "bull", name: "emails" }, id: "2" },
    state: "failed",
  })
  store.add({
    jobRef: { queue: { prefix: "bull", name: "reports" }, id: "10" },
    state: "failed",
  })
}

describe("BulkActionBar — cross-queue (K>1) paths", () => {
  it("preview view renders per-queue breakdown", async () => {
    seedCrossQueueSelection()
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    fireEvent.click(screen.getByTestId("bulk-action-retry"))
    expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "preview")
    const breakdown = screen.getByTestId("bulk-action-bar-preview-breakdown")
    expect(breakdown.textContent).toContain("emails2")
    expect(breakdown.textContent).toContain("reports1")
  })

  it("executing fans out one bulkRetryJobs call per queue (orchestrator path)", async () => {
    mockedRetry.mockImplementation(async ({ jobIds }) => ({
      status: "ok",
      data: {
        action: "retry",
        total: jobIds.length,
        succeeded: jobIds.length,
        failed: [],
        cancelled: false,
      },
    }))
    seedCrossQueueSelection()
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    fireEvent.click(screen.getByTestId("bulk-action-retry"))
    expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "preview")
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-bar-execute"))
    })
    await waitFor(() => expect(mockedRetry).toHaveBeenCalledTimes(2))
    // The two calls' queueRef names should be the two distinct queues
    // (emails and reports), in some order.
    const calledQueueNames = mockedRetry.mock.calls.map(
      (c) => (c[0] as { queueRef: { name: string } }).queueRef.name,
    )
    expect(calledQueueNames.sort()).toEqual(["emails", "reports"])
  })

  it("complete view renders per-queue breakdown summary", async () => {
    mockedRetry.mockImplementation(async ({ queueRef: qr, jobIds }) => {
      if (qr.name === "emails") {
        return {
          status: "ok",
          data: {
            action: "retry",
            total: jobIds.length,
            succeeded: jobIds.length - 1,
            failed: [{ jobId: "2", reason: "not in failed state" }],
            cancelled: false,
          },
        }
      }
      return {
        status: "ok",
        data: {
          action: "retry",
          total: jobIds.length,
          succeeded: jobIds.length,
          failed: [],
          cancelled: false,
        },
      }
    })
    seedCrossQueueSelection()
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    fireEvent.click(screen.getByTestId("bulk-action-retry"))
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-bar-execute"))
    })
    await waitFor(() =>
      expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "complete"),
    )
    const breakdown = screen.getByTestId("bulk-action-bar-complete-breakdown")
    expect(breakdown.textContent).toContain("emails")
    expect(breakdown.textContent).toContain("1/2")
    expect(breakdown.textContent).toContain("1 failed")
    expect(breakdown.textContent).toContain("reports")
    expect(breakdown.textContent).toContain("1/1")
  })

  it("consolidates the message when ALL queues fail with connectionLocked", async () => {
    mockedRetry.mockResolvedValue({
      status: "error",
      error: {
        kind: "connectionLocked",
        message: "Connection prod is locked: Press cmd-shift-U to unlock for this session.",
        docUrl: null,
      },
    })
    seedCrossQueueSelection()
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    fireEvent.click(screen.getByTestId("bulk-action-retry"))
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-bar-execute"))
    })
    await waitFor(() =>
      expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "complete"),
    )
    // Consolidated locked message rather than per-queue breakdown.
    const locked = screen.getByTestId("bulk-action-bar-complete-locked")
    expect(locked.textContent).toContain("All 3 jobs blocked")
    expect(locked.textContent).toContain("cmd-shift-U")
    // Per-queue breakdown should NOT be rendered when all-locked
    // consolidation is active.
    expect(screen.queryByTestId("bulk-action-bar-complete-breakdown")).toBeNull()
  })
})

describe("BulkActionBar — protected connection", () => {
  it("asks for the queue name, then removes under a one-shot unlock", async () => {
    seedSelection(["1", "2"])
    mockedLockState.mockResolvedValue({ status: "ok", data: "locked" })
    mockedUnlock.mockResolvedValue({ status: "ok", data: "unlocked" })
    mockedLock.mockResolvedValue({ status: "ok", data: "locked" })
    mockedRemove.mockResolvedValue({
      status: "ok",
      data: { action: "remove", total: 2, succeeded: 2, failed: [], cancelled: false },
    })
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    await waitFor(() => expect(mockedLockState).toHaveBeenCalled())
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-remove"))
    })
    const execute = await screen.findByTestId("bulk-action-bar-execute")
    await waitFor(() => expect(screen.getByTestId("confirm-sheet-name")).toBeInTheDocument())
    expect(execute).toBeDisabled()
    fireEvent.change(screen.getByTestId("confirm-sheet-name"), { target: { value: "emails" } })
    await act(async () => {
      fireEvent.click(execute)
    })
    await waitFor(() =>
      expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "complete"),
    )
    expect(mockedUnlock).toHaveBeenCalledWith({ connectionId: "conn-x" })
    expect(mockedRemove).toHaveBeenCalledTimes(1)
    await waitFor(() => expect(mockedLock).toHaveBeenCalledWith({ connectionId: "conn-x" }))
  })

  it("Enter does not run a removal: Cancel has the focus", async () => {
    seedSelection(["1"])
    const Wrap = wrap()
    render(
      <Wrap>
        <BulkActionBar connectionId="conn-x" queueRef={queueRef} />
      </Wrap>,
    )
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-remove"))
    })
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }))
    })
    expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "preview")
    expect(mockedRemove).not.toHaveBeenCalled()
  })
})
