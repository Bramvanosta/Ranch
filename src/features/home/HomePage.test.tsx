/**
 * The home page: connections as cards, grouped, with live
 * health; search across groups; one collapse state with the sidebar; a
 * first-run screen when there is nothing yet.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
const navigateTo = vi.hoisted(() => vi.fn())
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigateTo,
  Link: ({
    children,
    to,
    params,
    ...rest
  }: {
    children: ReactNode
    to: string
    params?: Record<string, string>
  }) => (
    <a href={params ? to.replace("$connectionId", params.connectionId) : to} {...rest}>
      {children}
    </a>
  ),
}))
// The edit dialog has its own tests.
vi.mock("@/features/connections/EditConnectionDialog", () => ({
  EditConnectionDialog: ({ open }: { open: boolean }) =>
    open ? <div data-testid="edit-dialog-stub" /> : null,
}))
// Dialogs and the move menu are covered by their own tests.
vi.mock("@/features/connections/useConnectionMenu", () => ({
  useConnectionMenu: (connection: { name: string }) => ({
    actions: () => [{ label: `menu of ${connection.name}` }],
    dialogs: null,
    openEdit: openEdit,
    openRemove: vi.fn(),
    duplicate: vi.fn(),
    isDuplicating: false,
  }),
}))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    listConnections: vi.fn(),
    listGroups: vi.fn(),
    listEnvironments: vi.fn(),
    listQueues: vi.fn(),
    getKvSetting: vi.fn(),
    setKvSetting: vi.fn(),
    getMcpSafetySettings: vi.fn(),
    getConnectionLockState: vi.fn(),
    setGroupCollapsed: vi.fn(),
    countQueueWorkers: vi.fn(),
    getThroughput: vi.fn(),
    openSettingsWindow: vi.fn(),
    getServerInfo: vi.fn(),
  },
}))

import { commands } from "@/lib/tauri/bindings"
import { useSidebarUiStore } from "@/stores/useSidebarUiStore"

import { HomePage } from "./HomePage"

const openEdit = vi.fn()
const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>
const ok = <T,>(data: T) => ({ status: "ok", data })

const connection = (id: string, name: string, over: Record<string, unknown> = {}) => ({
  id,
  environmentId: "e-staging",
  groupId: "g1",
  name,
  host: "redis.internal",
  port: 6379,
  db: 0,
  username: null,
  tls: false,
  tlsCustomCa: null,
  sshTunnel: null,
  envColor: "env-staging",
  envName: "staging",
  envLabel: "Staging",
  groupName: "Lumen",
  createdAt: "2026-09-18T10:00:00Z",
  mcpReadonly: null,
  ...over,
})

const queue = (counters: Record<string, number>, isPaused = false) => ({
  queueRef: { prefix: "bull", name: "q" },
  isPaused,
  counters: {
    waiting: 0,
    active: 0,
    completed: 0,
    failed: 0,
    delayed: 0,
    prioritized: 0,
    paused: 0,
    ...counters,
  },
})

function seed(connections: ReturnType<typeof connection>[]) {
  mocked.listConnections.mockResolvedValue(ok(connections))
  mocked.listGroups.mockResolvedValue(
    ok([
      { id: "g1", name: "Lumen", displayOrder: 0, collapsed: false, connectionCount: 2 },
      { id: "g2", name: "Side project", displayOrder: 1, collapsed: true, connectionCount: 1 },
    ]),
  )
  mocked.listEnvironments.mockResolvedValue(ok([]))
  mocked.getKvSetting.mockImplementation(async (key: string) =>
    ok(key === "selected_connection_id" ? "c2" : null),
  )
  mocked.getMcpSafetySettings.mockResolvedValue(
    ok({
      readonlyDefault: false,
      approvalMode: "writes_only",
      approvalTimeoutSeconds: 120,
      approvalAlwaysOnTop: true,
      approvalNotifySound: true,
    }),
  )
  mocked.getConnectionLockState.mockImplementation(
    async ({ connectionId }: { connectionId: string }) =>
      ok(connectionId === "c2" ? "locked" : "unlocked"),
  )
  mocked.setGroupCollapsed.mockResolvedValue(ok(null))
  mocked.countQueueWorkers.mockImplementation(async ({ queueRefs }: { queueRefs: unknown[] }) =>
    ok({ supported: true, counts: queueRefs.map((queueRef) => ({ queueRef, workers: 2 })) }),
  )
  mocked.getThroughput.mockResolvedValue(
    ok({
      range: "1h",
      bucketMs: 1,
      bucketStartsMs: [0],
      queues: [],
      completed: [10],
      failed: [4],
      metricsOffQueues: 0,
    }),
  )
  mocked.getServerInfo.mockResolvedValue(
    ok({
      redisVersion: "7.2.4",
      latencyMs: 3,
      usedMemoryBytes: null,
      maxMemoryBytes: null,
      opsPerSec: null,
      connectedClients: null,
    }),
  )
  mocked.listQueues.mockImplementation(async ({ connectionId }: { connectionId: string }) => {
    if (connectionId === "c2") {
      return {
        status: "error",
        error: { kind: "redis", message: "Connection refused", docUrl: null },
      }
    }
    return ok([queue({ waiting: 1200, active: 3, failed: 7 }), queue({ paused: 40 }, true)])
  })
}

function renderHome() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <HomePage />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  useSidebarUiStore.setState({ addConnectionTarget: null, newGroupRequested: false })
})

afterEach(() => {
  vi.resetAllMocks()
})

describe("HomePage", () => {
  const three = [
    connection("c1", "Queues"),
    connection("c2", "Queues prod", {
      environmentId: "e-prod",
      envColor: "env-production",
      envName: "production",
      envLabel: "Production",
      tls: true,
      mcpReadonly: true,
    }),
    connection("c3", "dev", {
      groupId: "g2",
      groupName: "Side project",
      environmentId: "e-local",
      envColor: "env-local",
      envName: "local",
      envLabel: "Local",
      host: "127.0.0.1",
      port: 6390,
    }),
  ]

  it("totals above one table per group, each connection with its health", async () => {
    seed(three)
    renderHome()
    expect(await screen.findByTestId("home-count")).toHaveTextContent("3 connections in 2 groups")

    const groups = screen.getAllByTestId("home-group")
    expect(groups.map((g) => g.getAttribute("aria-label"))).toEqual(["Lumen", "Side project"])
    // "Side project" is collapsed in the sidebar: collapsed here too.
    expect(within(groups[1] as HTMLElement).queryByTestId("fleet-row")).not.toBeInTheDocument()

    const rows = within(groups[0] as HTMLElement).getAllByTestId("fleet-row")
    expect(rows).toHaveLength(2)
    const healthy = rows[0] as HTMLElement
    expect(within(healthy).getByRole("link", { name: "Queues" })).toHaveAttribute(
      "href",
      "/connections/c1",
    )
    expect(healthy).toHaveTextContent("redis.internal:6379 · db 0")
    // 1200 + 40 parked waiting · 7 failed · 2 queues × 2 workers.
    await waitFor(() => expect(healthy).toHaveTextContent("1,240"))
    expect(healthy).toHaveTextContent("7")
    await waitFor(() =>
      expect(within(healthy).getByTestId("fleet-health")).toHaveTextContent("3 ms"),
    )

    await waitFor(() =>
      expect(screen.getByTestId("fleet-tile-connections")).toHaveTextContent("1 offline"),
    )
    // Two reachable connections (the folded group counts too): 7 + 7 failed, 4 + 4 in the hour.
    expect(screen.getByTestId("fleet-tile-failed")).toHaveTextContent("14+8 in the last hour")
    await waitFor(() => expect(screen.getByTestId("fleet-tile-workers")).toHaveTextContent("8"))
  })

  it("an unreachable connection says why in plain words, when it tries again, Retry now and Edit", async () => {
    seed(three)
    renderHome()
    await waitFor(() =>
      expect(screen.getAllByTestId("fleet-row")[1]).toHaveAttribute("data-offline", "true"),
    )
    const prod = screen.getAllByTestId("fleet-row")[1] as HTMLElement
    expect(within(prod).getByTestId("fleet-offline")).toHaveTextContent(
      "Connection refused. Is Redis running on port 6379?",
    )
    expect(within(prod).getByTestId("fleet-next-try")).toHaveTextContent(
      /next try in \d+ s|trying again/,
    )
    expect(prod).toHaveTextContent("TLS")
    expect(prod).toHaveTextContent("Agents read-only")
    expect(await within(prod).findByText("Read-only")).toBeInTheDocument()
    expect(prod).toHaveTextContent("last opened")

    const calls = mocked.listQueues.mock.calls.length
    fireEvent.click(within(prod).getByTestId("fleet-retry"))
    await waitFor(() => expect(mocked.listQueues.mock.calls.length).toBeGreaterThan(calls))
    fireEvent.click(within(prod).getByTestId("fleet-edit"))
    expect(screen.getByTestId("edit-dialog-stub")).toBeInTheDocument()
  })

  it("an empty group invites to add a connection there", async () => {
    seed(three)
    mocked.listGroups.mockResolvedValue(
      ok([
        { id: "g1", name: "Lumen", displayOrder: 0, collapsed: false, connectionCount: 2 },
        { id: "g2", name: "Side project", displayOrder: 1, collapsed: true, connectionCount: 1 },
        { id: "g3", name: "Client", displayOrder: 2, collapsed: false, connectionCount: 0 },
      ]),
    )
    renderHome()
    const empty = await screen.findByTestId("home-group-empty")
    expect(empty).toHaveTextContent("No connection in this group yet.")
    fireEvent.click(within(empty).getByRole("button", { name: "Add a connection here" }))
    expect(useSidebarUiStore.getState().addConnectionTarget).toEqual({
      groupId: "g3",
      environmentId: undefined,
    })
  })

  it("searches across groups — and through a collapsed one", async () => {
    seed(three)
    renderHome()
    fireEvent.change(await screen.findByTestId("home-search"), { target: { value: "6390" } })
    expect(screen.getByTestId("home-count")).toHaveTextContent("1 of 3 connections")
    const groups = screen.getAllByTestId("home-group")
    expect(groups.map((g) => g.getAttribute("aria-label"))).toEqual(["Side project"])
    expect(within(groups[0] as HTMLElement).getAllByTestId("fleet-row")).toHaveLength(1)

    fireEvent.change(screen.getByTestId("home-search"), { target: { value: "nope" } })
    expect(screen.getByTestId("home-no-match")).toHaveTextContent("No connection matches “nope”")
  })

  it("folding a group is the sidebar's persisted flag", async () => {
    seed(three)
    renderHome()
    fireEvent.click(await screen.findByRole("button", { name: /Lumen/ }))
    await waitFor(() =>
      expect(mocked.setGroupCollapsed).toHaveBeenCalledWith({ id: "g1", collapsed: true }),
    )
  })

  it("offers the page's actions", async () => {
    seed(three)
    renderHome()
    fireEvent.click(await screen.findByTestId("home-add-connection"))
    expect(useSidebarUiStore.getState().addConnectionTarget).toEqual({})
    fireEvent.click(screen.getByRole("button", { name: "New group" }))
    expect(useSidebarUiStore.getState().newGroupRequested).toBe(true)
  })

  it("is the first-run screen when there is no connection", async () => {
    seed([])
    renderHome()
    const welcome = await screen.findByTestId("home-first-run")
    expect(welcome).toHaveTextContent("Welcome to Ranch")
    // The Agents page, on its setup.
    fireEvent.click(within(welcome).getByRole("button", { name: /Set up agents/ }))
    expect(navigateTo).toHaveBeenCalledWith({ to: "/agents", search: { tab: "setup" } })
    fireEvent.click(screen.getByTestId("home-add-first"))
    expect(useSidebarUiStore.getState().addConnectionTarget).toEqual({})
    expect(mocked.listQueues).not.toHaveBeenCalled()
  })
})
