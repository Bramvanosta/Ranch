import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    listConnections: vi.fn(),
    listGroups: vi.fn(),
    listEnvironments: vi.fn(),
    createGroup: vi.fn(),
  },
}))
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn(), info: vi.fn() } }))
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  useParams: () => ({}),
  Link: ({ children, to }: { children: React.ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}))
vi.mock("./ReachabilityIndicator", () => ({ ReachabilityIndicator: () => null }))
vi.mock("./ConnectionSheet", () => ({
  ConnectionSheet: ({
    open,
    preferredPlacement,
  }: {
    open: boolean
    preferredPlacement?: unknown
  }) =>
    open ? (
      <div data-testid="connection-form">{JSON.stringify(preferredPlacement ?? null)}</div>
    ) : null,
}))

import { commands } from "@/lib/tauri/bindings"
import { useSidebarUiStore } from "@/stores/useSidebarUiStore"

import { ConnectionSidebar } from "./ConnectionSidebar"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>

const group = (id: string, name: string, connectionCount: number) => ({
  id,
  name,
  displayOrder: 0,
  collapsed: false,
  connectionCount,
})
const env = (id: string, groupId: string, label: string, colorToken = "env-local") => ({
  id,
  groupId,
  groupName: "",
  label,
  name: label,
  colorToken,
})
const connection = (id: string, name: string, environmentId: string, envLabel: string) => ({
  id,
  environmentId,
  groupId: "g1",
  name,
  host: "redis.lumen.internal",
  port: 6379,
  db: 0,
  username: null,
  tls: false,
  tlsCustomCa: null,
  sshTunnel: null,
  envColor: envLabel === "Staging" ? "env-staging" : "env-local",
  envName: envLabel,
  envLabel,
  groupName: "Lumen Studios",
  createdAt: "2026-09-18T00:00:00Z",
  mcpReadonly: null,
})

function seed() {
  mocked.listGroups.mockResolvedValue({
    status: "ok",
    data: [group("g1", "Lumen Studios", 3), group("g2", "Atelier", 0)],
  })
  mocked.listEnvironments.mockResolvedValue({
    status: "ok",
    data: [env("e-local", "g1", "Local"), env("e-staging", "g1", "Staging", "env-staging")],
  })
  mocked.listConnections.mockResolvedValue({
    status: "ok",
    data: [
      connection("c3", "lumen-api", "e-staging", "Staging"),
      connection("c2", "workers", "e-local", "Local"),
      connection("c1", "lumen-api", "e-local", "Local"),
    ],
  })
}

afterEach(() => {
  vi.resetAllMocks()
  act(() => {
    useSidebarUiStore.getState().closeAddConnection()
    useSidebarUiStore.getState().clearNewGroupRequest()
  })
})

function renderSidebar() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <ConnectionSidebar />
    </QueryClientProvider>,
  )
}

describe("ConnectionSidebar", () => {
  it("lists a group's connections directly, each with its environment tag", async () => {
    seed()
    renderSidebar()
    await screen.findAllByText("lumen-api")
    const rows = screen.getAllByRole("link").map((a) => a.textContent)
    // By environment (Local, then Staging), then by name; two connections share Local.
    expect(rows).toEqual(["lumen-apiLocal", "workersLocal", "lumen-apiStaging"])
    // No environment level: no row to fold, no "empty environment" placeholder.
    expect(screen.queryByText("Empty — drop a connection here")).not.toBeInTheDocument()
    expect(screen.getByText("Atelier")).toBeInTheDocument()
    expect(screen.getByText("No connections yet")).toBeInTheDocument()
  })

  it("the New group button shows an inline input that creates the group", async () => {
    seed()
    mocked.createGroup.mockResolvedValue({ status: "ok", data: group("g3", "Lumen Labs", 0) })
    renderSidebar()
    await screen.findAllByText("lumen-api")
    fireEvent.click(screen.getByRole("button", { name: "New group" }))
    const input = screen.getByRole("textbox", { name: "New group name" })
    fireEvent.change(input, { target: { value: "Lumen Labs" } })
    fireEvent.keyDown(input, { key: "Enter" })
    await waitFor(() => expect(mocked.createGroup).toHaveBeenCalledWith({ name: "Lumen Labs" }))
  })
})
