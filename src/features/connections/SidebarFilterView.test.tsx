import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

const navigate = vi.fn()
vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listConnections: vi.fn(), listGroups: vi.fn(), listQueues: vi.fn() },
}))
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigate,
  Link: ({ children, params }: { children: React.ReactNode; params: { queueName: string } }) => (
    <a href={`#${params.queueName}`} data-sidebar-row>
      {children}
    </a>
  ),
}))

import { commands } from "@/lib/tauri/bindings"
import { useCommandPaletteStore } from "@/stores/useCommandPaletteStore"
import { useSidebarFilterStore } from "@/stores/useSidebarFilterStore"

import { SidebarFilterField, SidebarFilterResults } from "./SidebarFilterView"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>
const ok = <T,>(data: T) => ({ status: "ok", data })
const conn = (id: string) => ({
  id,
  groupId: "g1",
  name: id,
  envColor: "env-local",
  envLabel: "Local",
})
const queue = (name: string) => ({
  id: `bull:${name}`,
  name,
  isPaused: false,
  counts: { waiting: 0, active: 0, completed: 0, failed: 0, delayed: 0 },
})

function renderSidebar() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <SidebarFilterField />
      <Results />
    </QueryClientProvider>,
  )
}
function Results() {
  const query = useSidebarFilterStore((s) => s.query)
  return query ? <SidebarFilterResults /> : null
}

describe("sidebar filter", () => {
  beforeEach(() => {
    mocked.listConnections.mockResolvedValue(ok([conn("local"), conn("prod")]))
    mocked.listGroups.mockResolvedValue(
      ok([{ id: "g1", name: "Lumen", displayOrder: 0, collapsed: false, connectionCount: 2 }]),
    )
    mocked.listQueues.mockImplementation(async ({ connectionId }: { connectionId: string }) =>
      ok(
        connectionId === "local"
          ? [queue("billing"), queue("emails")]
          : [queue("billing-retries"), queue("images")],
      ),
    )
  })
  afterEach(() => {
    useSidebarFilterStore.setState({ query: "" })
    useCommandPaletteStore.setState({ open: false, initialQuery: "" })
    navigate.mockReset()
  })

  it("shows the matching queues of every connection, highlighted, with a count", async () => {
    renderSidebar()
    fireEvent.change(screen.getByTestId("sidebar-filter"), { target: { value: "bill" } })
    expect(await screen.findByText("2 queues in 2 connections")).toBeInTheDocument()
    expect(screen.getAllByText("bill", { selector: "mark" })).toHaveLength(2)
    expect(screen.queryByText("emails")).toBeNull()
  })

  it("Enter opens the first match; ↓ moves into the matches", async () => {
    renderSidebar()
    const field = screen.getByTestId("sidebar-filter")
    fireEvent.change(field, { target: { value: "bill" } })
    await screen.findByText("2 queues in 2 connections")
    fireEvent.keyDown(field, { key: "ArrowDown" })
    expect(document.activeElement?.textContent).toBe("billing")
    fireEvent.keyDown(field, { key: "Enter" })
    expect(navigate).toHaveBeenCalledWith(
      expect.objectContaining({ params: { connectionId: "local", queueId: "bull:billing" } }),
    )
  })

  it("Enter hands a job ID to the palette", async () => {
    renderSidebar()
    const field = screen.getByTestId("sidebar-filter")
    fireEvent.change(field, { target: { value: "#1211" } })
    fireEvent.keyDown(field, { key: "Enter" })
    expect(useCommandPaletteStore.getState()).toMatchObject({ open: true, initialQuery: "#1211" })
    expect(navigate).not.toHaveBeenCalled()
  })

  it("Esc clears the filter", async () => {
    renderSidebar()
    const field = screen.getByTestId("sidebar-filter")
    fireEvent.change(field, { target: { value: "bill" } })
    await screen.findByText("2 queues in 2 connections")
    fireEvent.keyDown(field, { key: "Escape" })
    expect(useSidebarFilterStore.getState().query).toBe("")
    expect(screen.queryByTestId("sidebar-filter-count")).toBeNull()
  })
})
