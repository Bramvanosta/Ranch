import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    listPendingApprovals: vi.fn(),
    listApprovalHistory: vi.fn(),
    decidePendingApproval: vi.fn(),
    getConnectionLockState: vi.fn(),
    openSettingsWindow: vi.fn(),
  },
}))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }))
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }))
vi.mock("@/hooks/useCopyToClipboard", () => ({ useCopyToClipboard: () => vi.fn() }))
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }))
// The other tabs have their own tests.
vi.mock("@/features/mcp/activity/McpActivityPanel", () => ({ McpActivityPanel: () => null }))
vi.mock("@/features/mcp/McpSafetySection", () => ({ McpSafetySection: () => null }))
vi.mock("@/features/settings/AgentsSetup", () => ({ AgentsSetup: () => null }))

import { accessOf } from "@/features/mcp/agentAccess"
import { useAiStore } from "@/lib/ai/aiFeatures"
import { commands, type PendingApproval } from "@/lib/tauri/bindings"

import { AgentsPage } from "./AgentsPage"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>
const ok = <T,>(data: T) => ({ status: "ok", data })

const pending: PendingApproval = {
  id: "a1",
  createdAt: new Date().toISOString(),
  clientHint: "Claude Desktop",
  connectionId: "c1",
  connectionName: "prod-redis",
  groupName: "Lumen",
  environmentName: "Production",
  envColor: "env-production",
  tool: "retry_job",
  kind: "write",
  argumentsJson: JSON.stringify({ jobId: "42" }),
  targetPreviewJson: JSON.stringify({ kind: "job", queue: "emails", jobId: "42", name: "send" }),
  timeoutSeconds: 120,
} as PendingApproval

afterEach(() => {
  vi.resetAllMocks()
  useAiStore.getState().setEnabled(true)
})

function wrap(node: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>)
}

describe("AgentsPage — Approvals", () => {
  it("shows a waiting request in full and decides it once", async () => {
    mocked.listPendingApprovals.mockResolvedValue(ok([pending]))
    mocked.listApprovalHistory.mockResolvedValue(ok([]))
    mocked.getConnectionLockState.mockResolvedValue(ok("locked"))
    mocked.decidePendingApproval.mockResolvedValue(ok("decided"))
    wrap(<AgentsPage tab="approvals" onTab={vi.fn()} />)

    const card = await screen.findByTestId("agents-pending-approval")
    expect(within(card).getByText(/The agent gives up in/)).toBeInTheDocument()
    expect(within(card).getByTestId("mcp-approval-call")).toHaveTextContent("retry_job(")
    expect(await within(card).findByTestId("mcp-approval-protected")).toHaveTextContent(
      "prod-redis is protected",
    )
    fireEvent.click(within(card).getByRole("button", { name: "Approve once" }))
    await waitFor(() =>
      expect(mocked.decidePendingApproval).toHaveBeenCalledWith({
        approvalId: "a1",
        decision: "approve",
        reason: null,
      }),
    )
  })

  it("lists earlier decisions with who, what, where and how it ended", async () => {
    mocked.listPendingApprovals.mockResolvedValue(ok([]))
    mocked.listApprovalHistory.mockResolvedValue(
      ok([
        {
          id: "h1",
          createdAt: new Date().toISOString(),
          decidedAt: null,
          status: "denied",
          clientHint: "cursor",
          tool: "drain_queue",
          reason: "not now",
          connectionId: "c1",
          connectionName: "prod-redis",
        },
      ]),
    )
    wrap(<AgentsPage tab="approvals" onTab={vi.fn()} />)
    const row = await screen.findByTestId("agents-history-row")
    expect(row).toHaveTextContent("drain_queue")
    expect(row).toHaveTextContent("prod-redis")
    expect(row).toHaveTextContent("not now")
    expect(row).toHaveTextContent("Denied")
    expect(screen.getByText(/Nothing is waiting/)).toBeInTheDocument()
  })

  it("has the four tabs and asks for one", () => {
    mocked.listPendingApprovals.mockResolvedValue(ok([]))
    mocked.listApprovalHistory.mockResolvedValue(ok([]))
    const onTab = vi.fn()
    wrap(<AgentsPage tab="approvals" onTab={onTab} />)
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual([
      "Approvals",
      "Activity",
      "Notes",
      "Setup & safety",
    ])
    fireEvent.click(screen.getByRole("tab", { name: "Setup & safety" }))
    expect(onTab).toHaveBeenCalledWith("setup")
  })

  it("with AI features off, only says so and where to turn them on", () => {
    useAiStore.getState().setEnabled(false)
    wrap(<AgentsPage tab="approvals" onTab={vi.fn()} />)
    expect(screen.getByText("AI features are off")).toBeInTheDocument()
    expect(screen.queryByRole("tab")).not.toBeInTheDocument()
    expect(mocked.listPendingApprovals).not.toHaveBeenCalled()
  })
})

describe("accessOf", () => {
  it("is Read-only, else Ask or Free from the approval mode", () => {
    expect(accessOf(true, "writes_only")).toBe("Read-only")
    expect(accessOf(true, "off")).toBe("Read-only")
    expect(accessOf(false, "writes_only")).toBe("Ask")
    expect(accessOf(false, "all")).toBe("Ask")
    expect(accessOf(false, "off")).toBe("Free")
  })
})
