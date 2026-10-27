import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    listAttention: vi.fn(),
    getKvSetting: vi.fn(),
    setKvSetting: vi.fn(),
    listConnections: vi.fn(),
    getConnectionLockState: vi.fn(),
    unlockConnection: vi.fn(),
    lockConnection: vi.fn(),
    bulkRetryJobs: vi.fn(),
    resumeQueue: vi.fn(),
  },
}))
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, className }: { children: ReactNode; className?: string }) => (
    <a href="#x" className={className}>
      {children}
    </a>
  ),
}))
vi.mock("@/features/connections/EditConnectionDialog", () => ({
  EditConnectionDialog: () => <div data-testid="edit-dialog-stub" />,
}))
vi.mock("@/components/ui/dropdown-menu", () => ({
  DropdownMenu: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DropdownMenuTrigger: ({ children }: { children: ReactNode }) => <>{children}</>,
  DropdownMenuContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DropdownMenuItem: ({ children, onSelect }: { children: ReactNode; onSelect?: () => void }) => (
    <button type="button" onClick={onSelect}>
      {children}
    </button>
  ),
}))

import { type AttentionItem, commands } from "@/lib/tauri/bindings"

import { AttentionPage } from "./AttentionPage"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>
const q = (name: string) => ({ prefix: "bull", name })

const item = (over: Partial<AttentionItem>): AttentionItem => ({
  key: "k",
  kind: "failing",
  connectionId: "c-prod",
  connectionName: "prod",
  envColor: "env-production",
  envLabel: "Production",
  queueName: null,
  reason: "",
  sample: null,
  count: 0,
  sinceMs: Date.now() - 3_600_000,
  trend: [],
  jobs: [],
  ...over,
})

const ITEMS = [
  item({
    key: "failing:c-prod:x",
    reason: "connect ETIMEDOUT <ip>",
    count: 3,
    trend: [0, 1, 2],
    jobs: [
      { queue: q("emails"), id: "1" },
      { queue: q("emails"), id: "2" },
      { queue: q("images"), id: "9" },
    ],
  }),
  item({
    key: "paused:c-stage:emails",
    kind: "stuck",
    connectionId: "c-stage",
    connectionName: "staging",
    envColor: "env-staging",
    envLabel: "Staging",
    queueName: "emails",
    reason: "paused",
    count: 12,
  }),
  item({
    key: "offline:c-local",
    kind: "offline",
    connectionId: "c-local",
    connectionName: "local",
    envColor: "env-local",
    envLabel: "Local",
    reason: "Connection refused (os error 61)",
    sinceMs: null,
  }),
]

function wrap(children: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={client}>{children}</QueryClientProvider>)
}

let kv: string | null = null
beforeEach(() => {
  kv = null
  mocked.listAttention.mockResolvedValue({
    status: "ok",
    data: { items: ITEMS, connectionsChecked: 3 },
  })
  mocked.getKvSetting.mockImplementation(async () => ({ status: "ok", data: kv }))
  mocked.setKvSetting.mockImplementation(async (_k: string, v: string) => {
    kv = v
    return { status: "ok", data: null }
  })
  mocked.listConnections.mockResolvedValue({
    status: "ok",
    data: [{ id: "c-local", name: "local", host: "localhost", port: 6380 }],
  })
  mocked.getConnectionLockState.mockResolvedValue({ status: "ok", data: "unlocked" })
})
afterEach(() => vi.resetAllMocks())

describe("AttentionPage", () => {
  it("Failing, Stuck and Offline, each with where, since when and what to do", async () => {
    wrap(<AttentionPage />)
    const failing = await screen.findByTestId("attention-section-failing")
    expect(within(failing).getByTestId("attention-item-failing")).toHaveTextContent(
      "3 connect ETIMEDOUT <ip>",
    )
    expect(within(failing).getByTestId("attention-item-failing")).toHaveTextContent("since 1h ago")
    expect(within(failing).getByRole("img", { name: /Failures over 24 hours/ })).toBeInTheDocument()
    expect(within(failing).getByTestId("attention-retry")).toHaveTextContent("Retry 3")
    const stuck = screen.getByTestId("attention-section-stuck")
    expect(stuck).toHaveTextContent("paused · 12 waiting")
    expect(within(stuck).getByTestId("attention-resume")).toBeInTheDocument()
    const offline = screen.getByTestId("attention-section-offline")
    await waitFor(() =>
      expect(offline).toHaveTextContent("Connection refused. Is Redis running on port 6380?"),
    )
    expect(within(offline).getByTestId("attention-retry-now")).toBeInTheDocument()
    expect(within(offline).getByTestId("attention-edit")).toBeInTheDocument()
  })

  it("filters by environment and hides Local", async () => {
    wrap(<AttentionPage />)
    await screen.findByTestId("attention-item-failing")
    fireEvent.click(screen.getByTestId("attention-env-staging"))
    expect(screen.queryByTestId("attention-item-failing")).not.toBeInTheDocument()
    expect(screen.getByTestId("attention-item-stuck")).toBeInTheDocument()
    fireEvent.click(screen.getByTestId("attention-env-all"))
    fireEvent.click(screen.getByTestId("attention-hide-local"))
    expect(screen.queryByTestId("attention-item-offline")).not.toBeInTheDocument()
    expect(screen.getByTestId("attention-item-failing")).toBeInTheDocument()
  })

  it("Mute hides an item until then, and Muted (n) keeps it reachable", async () => {
    wrap(<AttentionPage />)
    const stuck = await screen.findByTestId("attention-item-stuck")
    fireEvent.click(within(stuck).getByText("For 1 day"))
    await waitFor(() =>
      expect(screen.queryByTestId("attention-item-stuck")).not.toBeInTheDocument(),
    )
    const saved = JSON.parse(kv ?? "{}") as Record<string, number>
    expect(saved["paused:c-stage:emails"]).toBeGreaterThan(Date.now() + 23 * 3_600_000)
    fireEvent.click(screen.getByTestId("attention-muted-toggle"))
    expect(screen.getByTestId("attention-muted-toggle")).toHaveTextContent("Muted (1)")
    expect(screen.getByTestId("attention-muted-item")).toHaveTextContent("staging / emails")
  })

  it("Retry N retries the cause's jobs, queue by queue, after the sheet", async () => {
    mocked.bulkRetryJobs.mockResolvedValue({
      status: "ok",
      data: { action: "retry", total: 2, succeeded: 2, failed: [], cancelled: false },
    })
    wrap(<AttentionPage />)
    fireEvent.click(await screen.findByTestId("attention-retry"))
    fireEvent.click(await screen.findByTestId("attention-confirm"))
    await waitFor(() => expect(mocked.bulkRetryJobs).toHaveBeenCalledTimes(2))
    expect(mocked.bulkRetryJobs).toHaveBeenCalledWith({
      connectionId: "c-prod",
      queueRef: q("emails"),
      jobIds: ["1", "2"],
    })
    expect(mocked.bulkRetryJobs).toHaveBeenCalledWith({
      connectionId: "c-prod",
      queueRef: q("images"),
      jobIds: ["9"],
    })
  })
})
