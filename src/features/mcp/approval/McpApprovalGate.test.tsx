import { type QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { formatMoment } from "@/lib/time"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    listPendingApprovals: vi.fn(),
    decidePendingApproval: vi.fn(),
    getMcpSafetySettings: vi.fn(),
  },
}))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }))
vi.mock("./approvalAttention", () => ({
  raiseAttention: vi.fn(() => Promise.resolve()),
  releaseAttention: vi.fn(() => Promise.resolve()),
}))
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }))
vi.mock("@/hooks/useCopyToClipboard", () => ({ useCopyToClipboard: () => vi.fn() }))

import { runCommand, useCommandHandler } from "@/commands/handlers"
import { mcpKeys } from "@/data/mcp/keys"
import { commands, type PendingApproval } from "@/lib/tauri/bindings"
import { useApprovalUiStore } from "@/stores/useApprovalUiStore"
import { testQueryClient } from "@/test/renderWithProviders"
import { raiseAttention, releaseAttention } from "./approvalAttention"
import { McpApprovalGate } from "./McpApprovalGate"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>

/** What `GlobalCommands` registers for Agents › Pending Approvals. */
function PendingApprovalsCommand() {
  useCommandHandler(
    "agents.pending",
    useApprovalUiStore((s) => s.showNow),
  )
  return null
}

const approval = (id: string, over: Partial<PendingApproval> = {}): PendingApproval => ({
  id,
  createdAt: new Date().toISOString(),
  clientHint: "Claude Desktop",
  connectionId: "c1",
  connectionName: "queues",
  groupName: "Lumen",
  environmentName: "Staging",
  envColor: "env-staging",
  tool: "retry_job",
  kind: "write",
  argumentsJson: JSON.stringify({ jobId: "42" }),
  targetPreviewJson: JSON.stringify({
    kind: "job",
    queue: "emails",
    jobId: "42",
    name: "send-welcome",
    state: "failed",
    attemptsMade: 3,
    failedReason: "connect ETIMEDOUT",
  }),
  timeoutSeconds: 120,
  ...over,
})

let client: QueryClient

function renderGate(pending: PendingApproval[], extra?: React.ReactNode) {
  mocked.listPendingApprovals.mockResolvedValue({ status: "ok", data: pending })
  mocked.getMcpSafetySettings.mockResolvedValue({
    status: "ok",
    data: {
      readonlyDefault: false,
      approvalMode: "writes_only",
      approvalTimeoutSeconds: 120,
      approvalAlwaysOnTop: true,
      approvalNotifySound: false,
    },
  })
  client = testQueryClient()
  render(
    <QueryClientProvider client={client}>
      {extra}
      <McpApprovalGate />
      <PendingApprovalsCommand />
    </QueryClientProvider>,
  )
}

/** What the backend watcher's event does: replace the pending list. */
function pushPending(pending: PendingApproval[]) {
  act(() => {
    client.setQueryData(mcpKeys.pendingApprovals, pending)
  })
}

afterEach(() => {
  vi.clearAllMocks()
  act(() => useApprovalUiStore.setState({ deferred: false, showRequest: 0 }))
})

describe("McpApprovalGate", () => {
  it("shows the oldest request with who, where, what, and focuses Deny, never Approve", async () => {
    renderGate([approval("a1"), approval("a2", { tool: "remove_job" })])

    const modal = await screen.findByTestId("mcp-approval-modal")
    expect(modal).toHaveTextContent("Claude Desktop wants to retry a job")
    expect(modal).toHaveTextContent("Lumen / Staging / queues")
    expect(modal).toHaveTextContent("retry_job")
    expect(screen.getByTestId("mcp-approval-preview")).toHaveTextContent("send-welcome")
    expect(screen.getByTestId("mcp-approval-preview")).toHaveTextContent("connect ETIMEDOUT")
    expect(screen.getByTestId("mcp-approval-countdown")).toHaveTextContent(/^1 of 2 · /)

    await waitFor(() => expect(screen.getByRole("button", { name: "Deny" })).toHaveFocus())
    expect(screen.getByRole("button", { name: "Approve once" })).not.toHaveFocus()
  })

  it("shows an agent's new job in full: queue, name, options and the payload", async () => {
    renderGate([
      approval("a1", {
        tool: "add_job",
        argumentsJson: JSON.stringify({ name: "refund-order" }),
        targetPreviewJson: JSON.stringify({
          kind: "newJob",
          queue: "payments",
          name: "refund-order",
          options: { attempts: 3, delay: 60000 },
          data: { orderId: 4711, amount: "199.00" },
        }),
      }),
    ])
    const modal = await screen.findByTestId("mcp-approval-modal")
    expect(modal).toHaveTextContent("Claude Desktop wants to add a job")
    const preview = screen.getByTestId("mcp-approval-preview")
    expect(preview).toHaveTextContent("refund-order")
    expect(preview).toHaveTextContent("new job in payments · attempts: 3 · delay: 60000")
    expect(preview).toHaveTextContent("orderId")
    expect(preview).toHaveTextContent("4711")
  })

  it("states how many jobs a drain touches, and marks it dangerous", async () => {
    renderGate([
      approval("a1", {
        tool: "drain_queue",
        argumentsJson: JSON.stringify({ queueId: { name: "emails" } }),
        targetPreviewJson: JSON.stringify({
          kind: "maintenance",
          queue: "emails",
          operation: { kind: "drain", includeDelayed: true },
          count: 41200,
          sparedSchedulerJobs: 1,
        }),
      }),
    ])
    const modal = await screen.findByTestId("mcp-approval-modal")
    expect(modal).toHaveTextContent("Claude Desktop wants to drain a queue")
    const preview = screen.getByTestId("mcp-approval-preview")
    expect(preview).toHaveTextContent("41,200 jobs in emails")
    expect(preview).toHaveTextContent("every waiting, paused, prioritized and delayed job")
    expect(preview).toHaveTextContent("1 scheduler run(s) left in place")
  })

  it("shows a scheduler change: pattern, next run and the data template", async () => {
    const nextRun = Date.UTC(2026, 8, 19, 1, 0, 0)
    renderGate([
      approval("a1", {
        tool: "upsert_scheduler",
        argumentsJson: JSON.stringify({ schedulerId: "nightly" }),
        targetPreviewJson: JSON.stringify({
          kind: "schedulerChange",
          queue: "reports",
          schedulerId: "nightly",
          jobName: "build-report",
          schedule: { kind: "pattern", pattern: "0 3 * * *", tz: "Europe/Paris" },
          nextRunMs: nextRun,
          data: { kind: "summary" },
        }),
      }),
    ])
    const modal = await screen.findByTestId("mcp-approval-modal")
    expect(modal).toHaveTextContent("Claude Desktop wants to create or change a scheduler")
    const preview = screen.getByTestId("mcp-approval-preview")
    expect(preview).toHaveTextContent("nightly → build-report")
    expect(preview).toHaveTextContent("scheduler in reports · 0 3 * * * (Europe/Paris)")
    expect(preview).toHaveTextContent(`Next run ${formatMoment(nextRun)}`)
    expect(preview).toHaveTextContent("summary")
  })

  it("Approve records the decision and moves on to the next request", async () => {
    mocked.decidePendingApproval.mockResolvedValue({ status: "ok", data: "decided" })
    renderGate([approval("a1"), approval("a2", { tool: "remove_job" })])
    await screen.findByTestId("mcp-approval-modal")

    fireEvent.click(screen.getByRole("button", { name: "Approve once" }))
    await waitFor(() =>
      expect(mocked.decidePendingApproval).toHaveBeenCalledWith({
        approvalId: "a1",
        decision: "approve",
        reason: null,
      }),
    )
    // Next one: a destructive tool: Approve is styled as dangerous.
    await waitFor(() =>
      expect(screen.getByTestId("mcp-approval-modal")).toHaveTextContent("wants to remove a job"),
    )
    expect(screen.getByRole("button", { name: "Approve once" }).className).toContain(
      "bg-(--danger)",
    )
  })

  it("Deny sends the typed reason", async () => {
    mocked.decidePendingApproval.mockResolvedValue({ status: "ok", data: "decided" })
    renderGate([approval("a1")])
    await screen.findByTestId("mcp-approval-modal")

    fireEvent.change(screen.getByLabelText(/Reason/), {
      target: { value: "  root cause not fixed " },
    })
    fireEvent.click(screen.getByRole("button", { name: "Deny" }))
    await waitFor(() =>
      expect(mocked.decidePendingApproval).toHaveBeenCalledWith({
        approvalId: "a1",
        decision: "deny",
        reason: "root cause not fixed",
      }),
    )
    await waitFor(() => expect(screen.queryByTestId("mcp-approval-modal")).not.toBeInTheDocument())
  })

  it("closing the modal denies: silence never approves", async () => {
    mocked.decidePendingApproval.mockResolvedValue({ status: "ok", data: "decided" })
    renderGate([approval("a1")])
    await screen.findByTestId("mcp-approval-modal")

    fireEvent.keyDown(screen.getByTestId("mcp-approval-modal"), { key: "Escape" })
    await waitFor(() =>
      expect(mocked.decidePendingApproval).toHaveBeenCalledWith({
        approvalId: "a1",
        decision: "deny",
        reason: "dismissed",
      }),
    )
  })

  it("a request that times out while open says so and records no decision", async () => {
    renderGate([approval("a1")])
    await screen.findByTestId("mcp-approval-modal")

    pushPending([])
    expect(await screen.findByText(/no longer waiting/)).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Approve once" })).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole("button", { name: "OK" }))
    await waitFor(() => expect(screen.queryByTestId("mcp-approval-modal")).not.toBeInTheDocument())
    expect(mocked.decidePendingApproval).not.toHaveBeenCalled()
  })

  it("raises attention once per request and releases it when the queue empties", async () => {
    renderGate([approval("a1")])
    await screen.findByTestId("mcp-approval-modal")
    await waitFor(() => expect(raiseAttention).toHaveBeenCalledTimes(1))
    expect(raiseAttention).toHaveBeenCalledWith(
      expect.objectContaining({
        alwaysOnTop: true,
        notifySound: false,
        body: expect.stringContaining("Lumen / Staging"),
      }),
    )

    pushPending([approval("a1"), approval("a2")])
    await waitFor(() => expect(raiseAttention).toHaveBeenCalledTimes(2))
    pushPending([approval("a1"), approval("a2")]) // same set again: no new alert
    expect(raiseAttention).toHaveBeenCalledTimes(2)

    pushPending([])
    await waitFor(() => expect(releaseAttention).toHaveBeenCalledTimes(1))
  })

  it("never stacks on another modal: waits until asked (Agents › Pending Approvals) or until it closes", async () => {
    renderGate([approval("a1")], <div role="dialog" data-testid="other-modal" />)

    expect(screen.queryByTestId("mcp-approval-modal")).not.toBeInTheDocument()

    act(() => {
      runCommand("agents.pending")
    })
    expect(await screen.findByTestId("mcp-approval-modal")).toBeInTheDocument()
  })
})
