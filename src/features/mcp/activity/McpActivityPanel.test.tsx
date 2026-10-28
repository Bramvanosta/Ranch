import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    listMcpActivity: vi.fn(),
    listMcpActivitySessions: vi.fn(),
    exportMcpActivity: vi.fn(),
    clearMcpActivity: vi.fn(),
    getKvSetting: vi.fn(),
    setKvSetting: vi.fn(),
  },
}))
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }))
vi.mock("@/hooks/useCopyToClipboard", () => ({ useCopyToClipboard: () => vi.fn() }))

import { commands } from "@/lib/tauri/bindings"

import { McpActivityPanel } from "./McpActivityPanel"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>

const event = (id: string, over: Record<string, unknown> = {}) => ({
  id,
  sessionId: "session-abcdef",
  timestamp: "2026-09-18T10:00:00.000Z",
  clientHint: "claude-desktop",
  tool: "list_jobs",
  kind: "read",
  connectionId: "c1",
  connectionName: "queues",
  targetJson: JSON.stringify({ connectionId: "c1", state: "failed" }),
  status: "success",
  durationMs: 12,
  error: null,
  approvalId: null,
  ...over,
})

afterEach(() => vi.resetAllMocks())

function renderPanel(events = [event("e1")]) {
  mocked.getKvSetting.mockResolvedValue({ status: "ok", data: null })
  mocked.listMcpActivity.mockResolvedValue({
    status: "ok",
    data: { events, total: events.length },
  })
  mocked.listMcpActivitySessions.mockResolvedValue({
    status: "ok",
    data: [
      {
        sessionId: "session-abcdef",
        clientHint: "claude-desktop",
        startedAt: "2026-09-18T10:00:00.000Z",
        lastAt: "2026-09-18T10:05:00.000Z",
        total: 14,
        writes: 2,
        notSuccessful: 1,
      },
    ],
  })
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <McpActivityPanel />
    </QueryClientProvider>,
  )
}

describe("McpActivityPanel", () => {
  it("lists reads and writes with an outcome that is more than a color", async () => {
    renderPanel([
      event("e2", {
        tool: "remove_job",
        kind: "write",
        status: "denied",
        error: "Denied by the user in Ranch",
        approvalId: "appr-1",
      }),
      event("e1"),
    ])
    const denied = await screen.findByTestId("activity-event-e2")
    expect(denied).toHaveTextContent("remove_job")
    expect(denied).toHaveTextContent("queues")
    expect(within(denied).getByTestId("activity-status-denied")).toHaveTextContent("Denied by you")
    expect(screen.getByTestId("activity-event-e1")).toHaveTextContent("Success")

    // Detail: error, approval, arguments.
    fireEvent.click(within(denied).getByRole("button", { expanded: false }))
    expect(denied).toHaveTextContent("Denied by the user in Ranch")
    expect(denied).toHaveTextContent("appr-1")
  })

  it("filters server-side", async () => {
    renderPanel()
    await screen.findByTestId("activity-event-e1")
    // All / Writes / Errors.
    fireEvent.click(screen.getByRole("radio", { name: "Writes" }))
    await waitFor(() =>
      expect(mocked.listMcpActivity).toHaveBeenLastCalledWith(
        expect.objectContaining({ filter: expect.objectContaining({ kind: "write" }), offset: 0 }),
      ),
    )
    fireEvent.click(screen.getByRole("radio", { name: "Errors" }))
    await waitFor(() =>
      expect(mocked.listMcpActivity).toHaveBeenLastCalledWith(
        expect.objectContaining({
          filter: expect.objectContaining({ kind: null, status: "error" }),
        }),
      ),
    )
  })

  it("Sessions → View narrows the events to that session", async () => {
    renderPanel()
    await screen.findByTestId("activity-event-e1")
    fireEvent.click(screen.getByRole("tab", { name: "Sessions" }))
    expect(await screen.findByText(/14 calls · 2 writes · 1 not successful/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole("button", { name: "View" }))
    await waitFor(() =>
      expect(mocked.listMcpActivity).toHaveBeenLastCalledWith(
        expect.objectContaining({
          filter: expect.objectContaining({ sessionId: "session-abcdef" }),
        }),
      ),
    )
  })

  it("exports with the current filter, clears, and toggles recording", async () => {
    mocked.exportMcpActivity.mockResolvedValue({ status: "ok", data: "[]" })
    mocked.clearMcpActivity.mockResolvedValue({ status: "ok", data: 1 })
    mocked.setKvSetting.mockResolvedValue({ status: "ok", data: null })
    URL.createObjectURL = vi.fn(() => "blob:x")
    URL.revokeObjectURL = vi.fn()
    renderPanel()
    await screen.findByTestId("activity-event-e1")

    fireEvent.click(screen.getByRole("button", { name: "Export CSV" }))
    await waitFor(() =>
      expect(mocked.exportMcpActivity).toHaveBeenCalledWith(
        expect.objectContaining({ format: "csv" }),
      ),
    )

    fireEvent.click(screen.getByRole("button", { name: "Clear" }))
    await waitFor(() => expect(mocked.clearMcpActivity).toHaveBeenCalled())

    fireEvent.click(screen.getByRole("switch", { name: "Record agent activity" }))
    await waitFor(() =>
      expect(mocked.setKvSetting).toHaveBeenCalledWith("mcp_activity_enabled", "false"),
    )
  })
})
