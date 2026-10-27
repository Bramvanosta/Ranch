/**
 * Queue maintenance: nothing runs before the affected count is
 * on screen, the right command runs with the chosen options, and a
 * locked connection disables the menu.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), warning: vi.fn(), error: vi.fn() } }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    previewQueueMaintenance: vi.fn(),
    drainQueue: vi.fn(),
    cleanQueue: vi.fn(),
    retryAllFailed: vi.fn(),
  },
}))

import { toast } from "sonner"

import { commands } from "@/lib/tauri/bindings"

import { QueueMaintenanceDialog } from "./QueueMaintenanceDialog"
import { QueueMaintenanceMenu } from "./QueueMaintenanceMenu"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>
const queueRef = { prefix: "bull", name: "emails" }

afterEach(() => {
  vi.resetAllMocks()
})

function wrap(ui: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>)
}

const previewOf = (affected: number, sparedSchedulerJobs = 0) => ({
  status: "ok",
  data: { affected, sparedSchedulerJobs },
})

describe("QueueMaintenanceMenu", () => {
  it("offers the three operations and opens the dialog of the one picked", async () => {
    mocked.previewQueueMaintenance.mockResolvedValue(previewOf(12))
    wrap(<QueueMaintenanceMenu connectionId="c1" queueRef={queueRef} locked={false} />)

    // Radix opens its menu from the keyboard (or pointerdown), not `click`.
    const trigger = screen.getByTestId("queue-maintenance-trigger")
    trigger.focus()
    fireEvent.keyDown(trigger, { key: "Enter" })
    expect(await screen.findByTestId("queue-maintenance-retry-all")).toBeInTheDocument()
    expect(screen.getByTestId("queue-maintenance-clean")).toBeInTheDocument()
    fireEvent.click(screen.getByTestId("queue-maintenance-drain"))

    expect(await screen.findByTestId("queue-maintenance-dialog")).toHaveTextContent("Drain emails")
    expect(mocked.previewQueueMaintenance).toHaveBeenCalledWith({
      connectionId: "c1",
      queueRef,
      operation: { kind: "drain", includeDelayed: true },
    })
  })

  // A locked connection asks for the queue's name in the sheet.
  it("stays available on a locked connection", () => {
    wrap(<QueueMaintenanceMenu connectionId="c1" queueRef={queueRef} locked />)
    expect(screen.getByTestId("queue-maintenance-trigger")).toBeEnabled()
  })
})

describe("QueueMaintenanceDialog", () => {
  it("cannot be confirmed before the count is shown, then drains with the chosen option", async () => {
    let resolvePreview: (v: unknown) => void = () => {}
    mocked.previewQueueMaintenance.mockReturnValueOnce(
      new Promise((r) => {
        resolvePreview = r
      }),
    )
    mocked.drainQueue.mockResolvedValue({
      status: "ok",
      data: { action: "drain", affected: 6, skipped: 1, cancelled: false },
    })
    const onClose = vi.fn()
    wrap(
      <QueueMaintenanceDialog
        kind="drain"
        onClose={onClose}
        connectionId="c1"
        queueRef={queueRef}
      />,
    )

    expect(screen.getByTestId("queue-maintenance-preview")).toHaveTextContent("Counting")
    expect(screen.getByTestId("queue-maintenance-confirm")).toBeDisabled()
    resolvePreview(previewOf(7, 1))
    expect(await screen.findByText(/7 jobs will be removed/)).toBeInTheDocument()
    expect(screen.getByTestId("queue-maintenance-preview")).toHaveTextContent(
      "The next run of a scheduler is left in place.",
    )

    // Changing an option re-counts.
    mocked.previewQueueMaintenance.mockResolvedValue(previewOf(5))
    fireEvent.click(screen.getByLabelText("Also remove delayed jobs"))
    expect(await screen.findByText(/5 jobs will be removed/)).toBeInTheDocument()
    expect(mocked.previewQueueMaintenance).toHaveBeenLastCalledWith(
      expect.objectContaining({ operation: { kind: "drain", includeDelayed: false } }),
    )

    fireEvent.click(screen.getByTestId("queue-maintenance-confirm"))
    await waitFor(() =>
      expect(mocked.drainQueue).toHaveBeenCalledWith({
        connectionId: "c1",
        queueRef,
        includeDelayed: false,
      }),
    )
    await waitFor(() => expect(onClose).toHaveBeenCalled())
    expect(toast.success).toHaveBeenCalledWith(
      "emails: 6 jobs removed · 1 left in place (flows, scheduler runs)",
    )
  })

  it("cleans the chosen state and age", async () => {
    mocked.previewQueueMaintenance.mockResolvedValue(previewOf(1200))
    mocked.cleanQueue.mockResolvedValue({
      status: "ok",
      data: { action: "clean", affected: 1200, skipped: 0, cancelled: false },
    })
    wrap(
      <QueueMaintenanceDialog
        kind="clean"
        onClose={vi.fn()}
        connectionId="c1"
        queueRef={queueRef}
      />,
    )
    fireEvent.change(screen.getByLabelText("Jobs"), { target: { value: "failed" } })
    fireEvent.change(screen.getByLabelText("Finished"), { target: { value: "86400000" } })
    await waitFor(() =>
      expect(mocked.previewQueueMaintenance).toHaveBeenLastCalledWith(
        expect.objectContaining({
          operation: { kind: "clean", state: "failed", olderThanMs: 86_400_000 },
        }),
      ),
    )
    const confirm = screen.getByTestId("queue-maintenance-confirm")
    await waitFor(() => expect(confirm).not.toBeDisabled())
    fireEvent.click(confirm)
    await waitFor(() =>
      expect(mocked.cleanQueue).toHaveBeenCalledWith({
        connectionId: "c1",
        queueRef,
        state: "failed",
        olderThanMs: 86_400_000,
      }),
    )
  })

  it("has nothing to confirm when no job is affected", async () => {
    mocked.previewQueueMaintenance.mockResolvedValue(previewOf(0))
    wrap(
      <QueueMaintenanceDialog
        kind="retryAllFailed"
        onClose={vi.fn()}
        connectionId="c1"
        queueRef={queueRef}
      />,
    )
    expect(await screen.findByText("No failed job to retry.")).toBeInTheDocument()
    expect(screen.getByTestId("queue-maintenance-confirm")).toBeDisabled()
  })

  it("keeps the dialog open and shows a refusal", async () => {
    mocked.previewQueueMaintenance.mockResolvedValue(previewOf(3))
    mocked.retryAllFailed.mockResolvedValue({
      status: "error",
      error: { kind: "connectionLocked", message: "Connection is locked", docUrl: null },
    })
    const onClose = vi.fn()
    wrap(
      <QueueMaintenanceDialog
        kind="retryAllFailed"
        onClose={onClose}
        connectionId="c1"
        queueRef={queueRef}
      />,
    )
    const confirm = screen.getByTestId("queue-maintenance-confirm")
    await waitFor(() => expect(confirm).not.toBeDisabled())
    fireEvent.click(confirm)
    expect(await screen.findByRole("alert")).toHaveTextContent("Connection is locked")
    expect(onClose).not.toHaveBeenCalled()
    expect(toast.success).not.toHaveBeenCalled()
  })
})
