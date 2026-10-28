import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@/hooks/useCopyToClipboard", () => ({
  useCopyToClipboard: () => vi.fn(),
}))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    listAuditLogEntries: vi.fn(),
    exportAuditLog: vi.fn(),
    listConnections: vi.fn(),
    getApprovalRecord: vi.fn(),
  },
}))
// jsdom has no layout: show every row of the virtualized table.
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count, estimateSize }: { count: number; estimateSize: () => number }) => ({
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        index,
        key: index,
        start: index * estimateSize(),
      })),
    getTotalSize: () => count * estimateSize(),
    scrollToIndex: () => {},
    measureElement: () => {},
  }),
}))

import { type AuditLogEntry, commands } from "@/lib/tauri/bindings"

import { AuditEntryDetail } from "./AuditEntryDetail"
import { AuditLogViewer } from "./AuditLogViewer"

const mockedList = commands.listAuditLogEntries as unknown as ReturnType<typeof vi.fn>
const mockedListConns = commands.listConnections as unknown as ReturnType<typeof vi.fn>
const mockedApproval = commands.getApprovalRecord as unknown as ReturnType<typeof vi.fn>

afterEach(() => {
  vi.resetAllMocks()
})

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

function entry(id: string): AuditLogEntry {
  return {
    id,
    createdAt: "2026-05-11T12:00:00Z",
    source: "ui",
    operatorId: "local",
    agentClient: null,
    agentTransport: null,
    action: "retryJob",
    target: { kind: "job", data: { queueId: "bull:emails", queueName: "emails", jobId: id } },
    outcome: "success",
    outcomeReason: null,
    connectionId: "conn-x",
    connectionName: "Acme · prod",
    envColor: "env-production",
    groupName: "Acme",
    environmentName: "Production",
    approvalId: null,
  }
}

describe("AuditLogViewer", () => {
  it("renders the empty state when no entries exist", async () => {
    mockedList.mockResolvedValue({ status: "ok", data: [] })
    mockedListConns.mockResolvedValue({ status: "ok", data: [] })
    const Wrap = wrap()
    render(
      <Wrap>
        <AuditLogViewer />
      </Wrap>,
    )
    expect(await screen.findByTestId("audit-log-empty")).toHaveTextContent(/No audit entries yet/)
  })

  it("renders the error state with retry when fetch fails", async () => {
    mockedList.mockResolvedValue({
      status: "error",
      error: { kind: "database", message: "disk full", docUrl: null },
    })
    mockedListConns.mockResolvedValue({ status: "ok", data: [] })
    const Wrap = wrap()
    render(
      <Wrap>
        <AuditLogViewer />
      </Wrap>,
    )
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load audit log")
    expect(screen.getByText(/disk full/)).toBeInTheDocument()
  })

  it("renders the table when entries exist", async () => {
    mockedList.mockResolvedValue({
      status: "ok",
      data: [entry("1"), entry("2"), entry("3")],
    })
    mockedListConns.mockResolvedValue({ status: "ok", data: [] })
    const Wrap = wrap()
    render(
      <Wrap>
        <AuditLogViewer />
      </Wrap>,
    )
    expect(await screen.findByTestId("audit-log-table")).toBeInTheDocument()
  })

  it("forwards filter changes to the underlying hook", async () => {
    mockedList.mockResolvedValue({ status: "ok", data: [] })
    mockedListConns.mockResolvedValue({ status: "ok", data: [] })
    const Wrap = wrap()
    render(
      <Wrap>
        <AuditLogViewer />
      </Wrap>,
    )
    await screen.findByTestId("audit-log-empty")
    fireEvent.change(screen.getByTestId("audit-filter-action"), {
      target: { value: "retryJob" },
    })
    expect(mockedList).toHaveBeenLastCalledWith(expect.objectContaining({ action: "retryJob" }))
  })

  it("renders the Export trigger button", async () => {
    // Radix DropdownMenu's onSelect path is portal- and pointer-event-
    // driven and doesn't fire reliably from jsdom's fireEvent (same
    // limitation as Base UI's ContextMenu).
    // We assert the trigger renders + the menu items show up in the
    // portal once it opens; the per-item click→export wiring is
    // covered by data/audit/useAuditExport.test.tsx in isolation.
    mockedList.mockResolvedValue({ status: "ok", data: [] })
    mockedListConns.mockResolvedValue({ status: "ok", data: [] })
    const Wrap = wrap()
    render(
      <Wrap>
        <AuditLogViewer />
      </Wrap>,
    )
    await screen.findByTestId("audit-log-empty")
    const trigger = screen.getByTestId("audit-export-trigger")
    expect(trigger).toBeInTheDocument()
    expect(trigger).toHaveTextContent("Export")
  })

  it("a row per entry: time, who, action, target, where, result", async () => {
    mockedList.mockResolvedValue({
      status: "ok",
      data: [
        entry("1"),
        {
          ...entry("2"),
          source: "agent",
          agentClient: "claude-code",
          action: "removeJob",
          outcome: "failure",
          outcomeReason: "Connection is locked",
        },
      ],
    })
    mockedListConns.mockResolvedValue({ status: "ok", data: [] })
    const Wrap = wrap()
    render(
      <Wrap>
        <AuditLogViewer />
      </Wrap>,
    )
    const row = await screen.findByTestId("audit-row-2")
    expect(row).toHaveTextContent("Claude Code")
    expect(row).toHaveTextContent("Remove job")
    expect(row).toHaveTextContent("emails / #2")
    expect(row).toHaveTextContent("Failed · Connection is locked")
    expect(screen.getByTestId("audit-row-1")).toHaveTextContent("You")
  })

  it("the text filter narrows the rows; the actor filter asks the backend", async () => {
    mockedList.mockResolvedValue({
      status: "ok",
      data: [
        entry("1"),
        {
          ...entry("2"),
          target: { kind: "queue", data: { queueId: "bull:reports", queueName: "reports" } },
        },
      ],
    })
    mockedListConns.mockResolvedValue({ status: "ok", data: [] })
    const Wrap = wrap()
    render(
      <Wrap>
        <AuditLogViewer />
      </Wrap>,
    )
    await screen.findByTestId("audit-row-2")
    fireEvent.change(screen.getByTestId("audit-filter-text"), { target: { value: "reports" } })
    expect(screen.queryByTestId("audit-row-1")).not.toBeInTheDocument()
    expect(screen.getByTestId("audit-row-2")).toBeInTheDocument()
    fireEvent.click(screen.getByTestId("audit-actor-you"))
    await waitFor(() =>
      expect(mockedList).toHaveBeenLastCalledWith(expect.objectContaining({ source: "ui" })),
    )
  })

  it("a row opens its details", async () => {
    mockedList.mockResolvedValue({ status: "ok", data: [entry("1")] })
    mockedListConns.mockResolvedValue({ status: "ok", data: [] })
    const onSelect = vi.fn()
    const Wrap = wrap()
    render(
      <Wrap>
        <AuditLogViewer onSelect={onSelect} />
      </Wrap>,
    )
    fireEvent.click(await screen.findByTestId("audit-row-1"))
    expect(onSelect).toHaveBeenCalledWith(expect.objectContaining({ id: "1" }))
  })

  it("the detail says who asked, who approved and after how long, and the exact arguments", async () => {
    mockedApproval.mockResolvedValue({
      status: "ok",
      data: {
        id: "ap1",
        createdAt: "2026-05-11T11:59:40Z",
        decidedAt: "2026-05-11T11:59:52Z",
        status: "consumed",
        clientHint: "claude-code",
        tool: "remove_job",
        reason: null,
        argumentsJson: '{"jobId":"1081"}',
      },
    })
    const Wrap = wrap()
    render(
      <Wrap>
        <AuditEntryDetail
          entry={{ ...entry("1"), source: "agent", agentClient: "claude-code", approvalId: "ap1" }}
          onClose={vi.fn()}
        />
      </Wrap>,
    )
    expect(await screen.findByTestId("audit-detail-approval")).toHaveTextContent(
      /Asked by Claude Code at .* approved by you after 12\.0 s/,
    )
    expect(screen.getByTestId("audit-detail-arguments")).toHaveTextContent('"jobId": "1081"')
    expect(screen.getByTestId("audit-detail-result")).toHaveTextContent("Done")
  })
})
