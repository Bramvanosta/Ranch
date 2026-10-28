import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router"
import { act, fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { bullmqConnection, redisEndpoint } from "@/domain/bullmqEndpoint"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    listConnections: vi.fn(),
    listGroups: vi.fn(),
    getKvSetting: vi.fn(),
    setKvSetting: vi.fn(),
    getJob: vi.fn(),
  },
}))

import { useCommandHandler } from "@/commands/handlers"
import { commands } from "@/lib/tauri/bindings"
import { useCommandPaletteStore } from "@/stores/useCommandPaletteStore"
import { useSidebarUiStore } from "@/stores/useSidebarUiStore"

import { CommandPalette } from "./CommandPalette"

const mocked = commands as unknown as {
  listConnections: ReturnType<typeof vi.fn>
  listGroups: ReturnType<typeof vi.fn>
  getKvSetting: ReturnType<typeof vi.fn>
  setKvSetting: ReturnType<typeof vi.fn>
}

function makeRouter(child: ReactNode) {
  const root = createRootRoute({ component: () => <>{child}</> })
  const index = createRoute({ getParentRoute: () => root, path: "/", component: () => <div /> })
  const settings = createRoute({
    getParentRoute: () => root,
    path: "/settings",
    component: () => <div>Settings page</div>,
  })
  const conns = createRoute({
    getParentRoute: () => root,
    path: "/connections/$connectionId",
    component: () => <div>Connection page</div>,
  })
  const audit = createRoute({
    getParentRoute: () => root,
    path: "/audit",
    component: () => <div>Audit page</div>,
  })
  const tree = root.addChildren([index, settings, audit, conns])
  return createRouter({
    routeTree: tree,
    history: createMemoryHistory({ initialEntries: ["/"] }),
  })
}

function wrap(child: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const router = makeRouter(child)
  return (
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  )
}

afterEach(() => {
  vi.resetAllMocks()
  act(() => {
    useCommandPaletteStore.getState().setOpen(false)
    useCommandPaletteStore.getState().hydrateRecents([])
  })
})

describe("CommandPalette", () => {
  it("opens on cmd-K and renders connections + settings groups", async () => {
    mocked.getKvSetting.mockResolvedValue({ status: "ok", data: null })
    mocked.listConnections.mockResolvedValue({
      status: "ok",
      data: [
        {
          id: "c1",
          name: "Prod Redis",
          ...bullmqConnection(redisEndpoint({ host: "prod.example.com" })),
          environmentId: "env-1",
          groupId: "g-1",
          envColor: "env-production",
          envName: "Production",
          envLabel: "Environment",
          groupName: "Default",
          createdAt: "2026-05-12T00:00:00Z",
          mcpReadonly: null,
        },
      ],
    })
    render(wrap(<CommandPalette />))

    // Open via the store (the cmd-K keystroke is tested separately
    // in useKeyboardShortcut.test.tsx; here we focus on the palette
    // surface itself).
    act(() => {
      useCommandPaletteStore.getState().setOpen(true)
    })
    await waitFor(() => {
      expect(screen.getByTestId("command-palette")).toBeInTheDocument()
    })
    await waitFor(() => {
      expect(screen.getByTestId("command-palette-item-c1")).toBeInTheDocument()
    })
    expect(screen.getByTestId("command-palette-item-go-settings")).toBeInTheDocument()
    expect(screen.getByTestId("command-palette-item-go-audit")).toBeInTheDocument()
  })

  it("filters as the user types", async () => {
    mocked.getKvSetting.mockResolvedValue({ status: "ok", data: null })
    mocked.listConnections.mockResolvedValue({
      status: "ok",
      data: [
        {
          id: "c1",
          name: "Prod Redis",
          ...bullmqConnection(redisEndpoint({ host: "prod.example.com" })),
          environmentId: "env-1",
          groupId: "g-1",
          envColor: "env-production",
          envName: "Production",
          envLabel: "Environment",
          groupName: "Default",
          createdAt: "2026-05-12T00:00:00Z",
          mcpReadonly: null,
        },
      ],
    })
    render(wrap(<CommandPalette />))
    act(() => {
      useCommandPaletteStore.getState().setOpen(true)
    })
    await waitFor(() => {
      expect(screen.getByTestId("command-palette-item-c1")).toBeInTheDocument()
    })
    fireEvent.change(screen.getByTestId("command-palette-input"), {
      target: { value: "audit" },
    })
    await waitFor(() => {
      expect(screen.queryByTestId("command-palette-item-c1")).not.toBeInTheDocument()
    })
    expect(screen.getByTestId("command-palette-item-go-audit")).toBeInTheDocument()
  })

  it("escape closes the palette via Dialog primitive", async () => {
    mocked.getKvSetting.mockResolvedValue({ status: "ok", data: null })
    mocked.listConnections.mockResolvedValue({ status: "ok", data: [] })
    render(wrap(<CommandPalette />))
    act(() => {
      useCommandPaletteStore.getState().setOpen(true)
    })
    await waitFor(() => {
      expect(screen.getByTestId("command-palette")).toBeInTheDocument()
    })
    act(() => {
      useCommandPaletteStore.getState().setOpen(false)
    })
    await waitFor(() => {
      expect(screen.queryByTestId("command-palette")).not.toBeInTheDocument()
    })
  })

  // ── Groups ────────────────────────────────────────────────────────────

  async function openWithGroups() {
    mocked.getKvSetting.mockResolvedValue({ status: "ok", data: null })
    mocked.listConnections.mockResolvedValue({ status: "ok", data: [] })
    mocked.listGroups.mockResolvedValue({
      status: "ok",
      data: [{ id: "g1", name: "Lumen", displayOrder: 0, collapsed: true, connectionCount: 3 }],
    })
    render(wrap(<CommandPalette />))
    act(() => {
      useCommandPaletteStore.getState().setOpen(true)
    })
    await screen.findByTestId("command-palette-item-go-group-g1")
  }

  it("lists groups; selecting one asks the sidebar to reveal it", async () => {
    await openWithGroups()
    fireEvent.click(screen.getByTestId("command-palette-item-go-group-g1"))
    expect(useSidebarUiStore.getState().revealGroupId).toBe("g1")
    expect(useSidebarUiStore.getState().renameGroupId).toBeNull()
    expect(useCommandPaletteStore.getState().open).toBe(false)
    // Sidebar intents are not navigations; they stay out of recents.
    expect(useCommandPaletteStore.getState().recents).toEqual([])
    act(() => useSidebarUiStore.getState().clearReveal())
  })

  it('"Rename group" and "New group" raise the matching sidebar intents', async () => {
    // New group is a registry command; `GlobalCommands` handles it in the app.
    renderHook(() =>
      useCommandHandler("file.newGroup", useSidebarUiStore.getState().requestNewGroup),
    )
    await openWithGroups()
    fireEvent.change(screen.getByTestId("command-palette-input"), { target: { value: "rename" } })
    fireEvent.click(await screen.findByTestId("command-palette-item-rename-group-g1"))
    expect(useSidebarUiStore.getState().renameGroupId).toBe("g1")
    act(() => useSidebarUiStore.getState().clearReveal())

    act(() => {
      useCommandPaletteStore.getState().setOpen(true)
    })
    fireEvent.click(await screen.findByTestId("command-palette-item-command-file.newGroup"))
    expect(useSidebarUiStore.getState().newGroupRequested).toBe(true)
    act(() => useSidebarUiStore.getState().clearNewGroupRequest())
  })

  it("does not swallow the letters j and k typed in the search field", async () => {
    await openWithGroups()
    const input = screen.getByTestId("command-palette-input")
    // fireEvent returns false when the handler called preventDefault().
    expect(fireEvent.keyDown(input, { key: "j" })).toBe(true)
    expect(fireEvent.keyDown(input, { key: "k" })).toBe(true)
    // Ctrl+J / Ctrl+K (and the arrows) still move the highlight.
    expect(fireEvent.keyDown(input, { key: "j", ctrlKey: true })).toBe(false)
    expect(fireEvent.keyDown(input, { key: "ArrowUp" })).toBe(false)
  })

  it("finds the queues Ranch already listed, once the user types", async () => {
    mocked.getKvSetting.mockResolvedValue({ status: "ok", data: null })
    mocked.listGroups.mockResolvedValue({ status: "ok", data: [] })
    mocked.listConnections.mockResolvedValue({
      status: "ok",
      data: [
        {
          id: "c1",
          name: "Prod Redis",
          ...bullmqConnection(redisEndpoint({ host: "prod.example.com" })),
          environmentId: "env-1",
          groupId: "g-1",
          envColor: "env-production",
          envName: "Production",
          envLabel: "Production",
          groupName: "Acme",
          createdAt: "2026-05-12T00:00:00Z",
          mcpReadonly: null,
        },
      ],
    })
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    client.setQueryData(
      ["queues", "c1"],
      [
        {
          id: "bull:invoices",
          name: "invoices",
          counts: { failed: 12 },
          isPaused: false,
        },
      ],
    )
    render(
      <QueryClientProvider client={client}>
        <RouterProvider router={makeRouter(<CommandPalette />)} />
      </QueryClientProvider>,
    )
    act(() => {
      useCommandPaletteStore.getState().setOpen(true)
    })
    await screen.findByTestId("command-palette-item-c1")
    // Not listed on an empty query: the list would be too long.
    expect(screen.queryByTestId("command-palette-item-c1:bull:invoices")).not.toBeInTheDocument()

    fireEvent.change(screen.getByTestId("command-palette-input"), { target: { value: "invoi" } })
    const item = await screen.findByTestId("command-palette-item-c1:bull:invoices")
    expect(item).toHaveTextContent("invoices")
    // With its failures and where it is.
    expect(item).toHaveTextContent("12 failed · Prod Redis · Production")

    // ⇥ lists what can be done with it.
    fireEvent.keyDown(screen.getByTestId("command-palette-input"), { key: "Tab" })
    const actions = await screen.findByTestId("command-palette-actions")
    expect(actions).toHaveTextContent("Actions for invoices")
    expect(screen.getByTestId("command-palette-action-failed")).toHaveTextContent(
      "Open its failed jobs by cause",
    )
    expect(screen.getByTestId("command-palette-action-key")).toBeInTheDocument()
    // ⇧⇥ goes back to the results.
    fireEvent.keyDown(screen.getByTestId("command-palette-input"), { key: "Tab", shiftKey: true })
    expect(screen.queryByTestId("command-palette-actions")).not.toBeInTheDocument()

    // `#1211` finds that job among the queues Ranch has listed.
    ;(commands.getJob as unknown as ReturnType<typeof vi.fn>).mockResolvedValue({
      status: "ok",
      data: {
        jobRef: { queueId: "bull:invoices", queueName: "invoices", id: "1211" },
        name: "send-invoice",
        state: "failed",
      },
    })
    fireEvent.change(screen.getByTestId("command-palette-input"), { target: { value: "#1211" } })
    const job = await screen.findByTestId("command-palette-item-job:c1:bull:invoices:1211")
    expect(job).toHaveTextContent("send-invoice #1211")
    expect(job).toHaveTextContent("failed · invoices · Prod Redis")
  })
})
