import { act, fireEvent, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

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

import { commands } from "@/lib/tauri/bindings"
import { useBulkProgressStore } from "@/stores/useBulkProgressStore"
import { useJobSelectionStore } from "@/stores/useJobSelectionStore"
import { aConnection, aQueueId } from "@/test/factories"
import { pressChord } from "@/test/pressChord"
import { renderWithProviders } from "@/test/renderWithProviders"
import { err, ok } from "@/test/results"

import { BulkActionBar } from "./BulkActionBar"

const mockedRetry = vi.mocked(commands.bulkRetryJobs)
const mockedRemove = vi.mocked(commands.bulkRemoveJobs)
const mockedListConns = vi.mocked(commands.listConnections)
const mockedLockState = vi.mocked(commands.getConnectionLockState)
const mockedUnlock = vi.mocked(commands.unlockConnection)
const mockedLock = vi.mocked(commands.lockConnection)

const queueId = aQueueId()

function renderBar() {
  return renderWithProviders(<BulkActionBar connectionId="conn-x" queueId={queueId} />)
}

beforeEach(() => {
  Object.defineProperty(navigator, "platform", { value: "MacIntel", configurable: true })
  useJobSelectionStore.getState().clear()
  useBulkProgressStore.getState().finish()
  mockedListConns.mockResolvedValue(ok([aConnection({ id: "conn-x" })]))
  mockedLockState.mockResolvedValue(ok("unlocked"))
})

afterEach(() => {
  vi.resetAllMocks()
})

function seedSelection(jobIds: string[], queueName = "emails") {
  const store = useJobSelectionStore.getState()
  for (const id of jobIds) {
    store.add({
      jobRef: { queueId: `bull:${queueName}`, queueName: queueName, id },
      state: "failed",
    })
  }
}

describe("BulkActionBar: selecting state", () => {
  it("hides itself when selection is empty", () => {
    const { container } = renderBar()
    expect(container.querySelector("[data-testid='bulk-action-bar']")).toBeNull()
  })

  it("renders the count when jobs are selected", () => {
    seedSelection(["1", "2", "3"])
    renderBar()
    const bar = screen.getByTestId("bulk-action-bar")
    expect(bar).toHaveAttribute("data-state", "selecting")
    expect(screen.getByTestId("bulk-action-bar-count").textContent).toMatch(/3.*1.*queue/)
  })

  it("enables action buttons when selection spans multiple queues (K > 1)", () => {
    const store = useJobSelectionStore.getState()
    store.add({
      jobRef: { queueId: "bull:emails", queueName: "emails", id: "1" },
      state: "failed",
    })
    store.add({
      jobRef: { queueId: "bull:reports", queueName: "reports", id: "1" },
      state: "failed",
    })
    renderBar()
    // K>1 goes through the frontend orchestrator, so the action
    // buttons are not disabled.
    expect(screen.getByTestId("bulk-action-retry")).not.toBeDisabled()
    expect(screen.getByTestId("bulk-action-remove")).not.toBeDisabled()
    expect(screen.getByTestId("bulk-action-promote")).not.toBeDisabled()
  })

  it("clicking Cancel clears the selection", async () => {
    seedSelection(["1", "2"])
    renderBar()
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-bar-cancel"))
    })
    expect(useJobSelectionStore.getState().selectedCount()).toBe(0)
  })
})

describe("BulkActionBar: preview state", () => {
  it("clicking Retry transitions to preview with the right banner text", async () => {
    seedSelection(["1", "2", "3"])
    renderBar()
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
    renderBar()
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-remove"))
    })
    expect(screen.getByTestId("bulk-action-bar-preview").textContent).toMatch(/irreversible/i)
  })

  it("an action requested from the list (cmd-Backspace on a row) opens the confirmation step", async () => {
    seedSelection(["7"])
    renderBar()
    await act(async () => {
      useJobSelectionStore.getState().requestAction("remove")
    })
    expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "preview")
    // One job: singular, and irreversible all the same.
    const preview = screen.getByTestId("bulk-action-bar-preview").textContent ?? ""
    expect(preview).toMatch(/Remove 1 job across 1 queue/)
    expect(preview).toMatch(/irreversible/i)
    expect(useJobSelectionStore.getState().requestedAction).toBeNull()
  })

  it("Escape from preview returns to selecting", async () => {
    seedSelection(["1"])
    renderBar()
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

describe("BulkActionBar: executing + complete states", () => {
  it("Enter on preview transitions to executing then complete on resolve", async () => {
    seedSelection(["1", "2"])
    mockedRetry.mockResolvedValue(
      ok({
        action: "retry",
        total: 2,
        succeeded: 2,
        failed: [],
        cancelled: false,
      }),
    )
    renderBar()
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
    mockedRetry.mockResolvedValue(
      err(
        "connectionLocked",
        "Connection prod is locked: Press cmd-shift-U to unlock for this session.",
      ),
    )
    renderBar()
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
    mockedRemove.mockResolvedValue(
      ok({
        action: "remove",
        total: 3,
        succeeded: 2,
        failed: [{ jobId: "3", reason: "not found" }],
        cancelled: false,
      }),
    )
    renderBar()
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

describe("BulkActionBar: keyboard shortcuts", () => {
  it("⇧⌘R from selecting transitions to retry preview", async () => {
    seedSelection(["1"])
    renderBar()
    await act(async () => {
      pressChord("cmd+shift+r")
    })
    expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "preview")
  })

  it("⇧⌘R opens preview when selection is cross-queue (K > 1)", async () => {
    const store = useJobSelectionStore.getState()
    store.add({
      jobRef: { queueId: "bull:emails", queueName: "emails", id: "1" },
      state: "failed",
    })
    store.add({
      jobRef: { queueId: "bull:reports", queueName: "reports", id: "1" },
      state: "failed",
    })
    renderBar()
    await act(async () => {
      pressChord("cmd+shift+r")
    })
    // K>1 cmd-R opens the preview too: the cross-queue orchestrator runs it.
    expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "preview")
  })
})

// ──────────────────────────────────────────────────────────────────────────
// cross-queue (K>1) BulkActionBar paths
// ──────────────────────────────────────────────────────────────────────────

function seedCrossQueueSelection() {
  const store = useJobSelectionStore.getState()
  store.add({
    jobRef: { queueId: "bull:emails", queueName: "emails", id: "1" },
    state: "failed",
  })
  store.add({
    jobRef: { queueId: "bull:emails", queueName: "emails", id: "2" },
    state: "failed",
  })
  store.add({
    jobRef: { queueId: "bull:reports", queueName: "reports", id: "10" },
    state: "failed",
  })
}

describe("BulkActionBar: cross-queue (K>1) paths", () => {
  it("preview view renders per-queue breakdown", async () => {
    seedCrossQueueSelection()
    renderBar()
    fireEvent.click(screen.getByTestId("bulk-action-retry"))
    expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "preview")
    const breakdown = screen.getByTestId("bulk-action-bar-preview-breakdown")
    expect(breakdown.textContent).toContain("emails2")
    expect(breakdown.textContent).toContain("reports1")
  })

  it("executing fans out one bulkRetryJobs call per queue (orchestrator path)", async () => {
    mockedRetry.mockImplementation(async ({ jobIds }) =>
      ok({
        action: "retry",
        total: jobIds.length,
        succeeded: jobIds.length,
        failed: [],
        cancelled: false,
      }),
    )
    seedCrossQueueSelection()
    renderBar()
    fireEvent.click(screen.getByTestId("bulk-action-retry"))
    expect(screen.getByTestId("bulk-action-bar")).toHaveAttribute("data-state", "preview")
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-action-bar-execute"))
    })
    await waitFor(() => expect(mockedRetry).toHaveBeenCalledTimes(2))
    // The two calls' queueId names should be the two distinct queues
    // (emails and reports), in some order.
    const calledQueueIds = mockedRetry.mock.calls.map(([args]) => args.queueId)
    expect(calledQueueIds.sort()).toEqual(["bull:emails", "bull:reports"])
  })

  it("complete view renders per-queue breakdown summary", async () => {
    mockedRetry.mockImplementation(async ({ queueId: qr, jobIds }) => {
      if (qr === "bull:emails") {
        return ok({
          action: "retry",
          total: jobIds.length,
          succeeded: jobIds.length - 1,
          failed: [{ jobId: "2", reason: "not in failed state" }],
          cancelled: false,
        })
      }
      return ok({
        action: "retry",
        total: jobIds.length,
        succeeded: jobIds.length,
        failed: [],
        cancelled: false,
      })
    })
    seedCrossQueueSelection()
    renderBar()
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
    mockedRetry.mockResolvedValue(
      err(
        "connectionLocked",
        "Connection prod is locked: Press cmd-shift-U to unlock for this session.",
      ),
    )
    seedCrossQueueSelection()
    renderBar()
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

describe("BulkActionBar: protected connection", () => {
  it("asks for the queue name, then removes under a one-shot unlock", async () => {
    seedSelection(["1", "2"])
    mockedLockState.mockResolvedValue(ok("locked"))
    mockedUnlock.mockResolvedValue(ok("unlocked"))
    mockedLock.mockResolvedValue(ok("locked"))
    mockedRemove.mockResolvedValue(
      ok({ action: "remove", total: 2, succeeded: 2, failed: [], cancelled: false }),
    )
    renderBar()
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
    renderBar()
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
