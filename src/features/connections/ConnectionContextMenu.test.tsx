import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    duplicateConnection: vi.fn(),
    changeConnectionColor: vi.fn(),
    removeConnection: vi.fn(),
    pingConnection: vi.fn(),
    moveConnection: vi.fn(),
    createEnvironment: vi.fn(),
    renameEnvironment: vi.fn(),
  },
}))
// Data behind the "Move to" submenu.
// Data behind the Environment and Move to Group submenus.
vi.mock("@/features/groups/useGroups", () => ({
  useGroups: () => ({
    data: [
      { id: "g1", name: "Lumen Studios", displayOrder: 0, collapsed: false, connectionCount: 1 },
      { id: "g2", name: "Lumen Labs", displayOrder: 1, collapsed: false, connectionCount: 0 },
      { id: "g3", name: "Atelier", displayOrder: 2, collapsed: false, connectionCount: 0 },
    ],
  }),
  useInvalidateHierarchy: () => () => {},
}))
const envRow = (id: string, groupId: string, label: string, colorToken: string) => ({
  id,
  groupId,
  groupName: "",
  label,
  name: label,
  colorToken,
})
vi.mock("./useEnvironments", () => ({
  useEnvironments: () => ({
    data: [
      envRow("e1", "g1", "Local", "env-local"),
      envRow("e2", "g1", "Production", "env-production"),
      envRow("e7", "g2", "Local", "env-local"),
    ],
  }),
}))
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn(), info: vi.fn() } }))
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  useParams: () => ({}),
  Link: ({ children, ...rest }: { children: React.ReactNode }) => (
    <a {...(rest as Record<string, unknown>)}>{children}</a>
  ),
}))
vi.mock("./ReachabilityIndicator", () => ({ ReachabilityIndicator: () => null }))
vi.mock("./EditConnectionDialog", () => ({
  EditConnectionDialog: ({ open }: { open: boolean }) =>
    open ? <div data-testid="edit-dialog" /> : null,
}))

import { commands } from "@/lib/tauri/bindings"

import { ConnectionContextMenu } from "./ConnectionContextMenu"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>

const connection = {
  id: "conn-1",
  environmentId: "e1",
  groupId: "g1",
  name: "lumen-api",
  host: "redis.lumen.internal",
  port: 6379,
  db: 0,
  username: null,
  tls: false,
  tlsCustomCa: null,
  sshTunnel: null,
  envColor: "env-local",
  envName: "Local",
  envLabel: "Local",
  groupName: "Lumen Studios",
  createdAt: "2026-05-03T00:00:00Z",
  mcpReadonly: null,
  sidebarCollapsed: false,
}

afterEach(() => vi.resetAllMocks())

function renderMenu() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <ul>
        <ConnectionContextMenu connection={connection} />
      </ul>
    </QueryClientProvider>,
  )
  fireEvent.contextMenu(screen.getByText("lumen-api"))
}

/**
 * Regression guard for the dead-menu bug: items were wired with Radix's
 * `onSelect`, which Base UI's `Menu.Item` does not implement — clicks did
 * nothing. These tests click the items instead of only asserting presence.
 */
describe("ConnectionContextMenu", () => {
  it("Edit opens the edit dialog", async () => {
    renderMenu()
    fireEvent.click(await screen.findByText("Edit Connection…"))
    expect(await screen.findByTestId("edit-dialog")).toBeInTheDocument()
  })

  it("Duplicate calls the backend", async () => {
    mocked.duplicateConnection.mockResolvedValue({
      status: "ok",
      data: { ...connection, id: "conn-2", name: "lumen-api (copy)" },
    })
    renderMenu()
    fireEvent.click(await screen.findByText("Duplicate"))
    await waitFor(() => expect(mocked.duplicateConnection).toHaveBeenCalledWith({ id: "conn-1" }))
  })

  it("Remove opens the confirmation dialog", async () => {
    renderMenu()
    fireEvent.click(await screen.findByText("Remove Connection…"))
    expect(await screen.findByText("Remove “lumen-api”?")).toBeInTheDocument()
  })

  it("Environment retags the connection within its group and reports a new lock", async () => {
    mocked.moveConnection.mockResolvedValue({ status: "ok", data: { newlyLocked: true } })
    renderMenu()
    fireEvent.click(await screen.findByText("Environment"))
    fireEvent.click(await screen.findByText("Production"))
    await waitFor(() =>
      expect(mocked.moveConnection).toHaveBeenCalledWith({
        connectionId: "conn-1",
        environmentId: "e2",
      }),
    )
    const { toast } = await import("sonner")
    await waitFor(() =>
      expect(toast.info).toHaveBeenCalledWith(expect.stringContaining("now locked")),
    )
  })

  it("the current environment and group are marked and disabled", async () => {
    renderMenu()
    fireEvent.click(await screen.findByText("Environment"))
    const current = await screen.findByText("Local (current)")
    expect(current.closest("[data-disabled]")).not.toBeNull()
  })

  it("Move to Group keeps the tag: into the same-named environment there", async () => {
    mocked.moveConnection.mockResolvedValue({ status: "ok", data: { newlyLocked: false } })
    renderMenu()
    fireEvent.click(await screen.findByText("Move to Group"))
    fireEvent.click(await screen.findByText("Lumen Labs"))
    await waitFor(() =>
      expect(mocked.moveConnection).toHaveBeenCalledWith({
        connectionId: "conn-1",
        environmentId: "e7",
      }),
    )
    expect(mocked.createEnvironment).not.toHaveBeenCalled()
  })

  it("Move to Group creates the tag in a group that does not have it", async () => {
    mocked.createEnvironment.mockResolvedValue({ status: "ok", data: "e-new" })
    mocked.moveConnection.mockResolvedValue({ status: "ok", data: { newlyLocked: false } })
    renderMenu()
    fireEvent.click(await screen.findByText("Move to Group"))
    fireEvent.click(await screen.findByText("Atelier"))
    await waitFor(() =>
      expect(mocked.createEnvironment).toHaveBeenCalledWith({
        groupId: "g3",
        name: "Local",
        colorToken: "env-local",
      }),
    )
    await waitFor(() =>
      expect(mocked.moveConnection).toHaveBeenCalledWith({
        connectionId: "conn-1",
        environmentId: "e-new",
      }),
    )
  })
})

describe("renaming an environment tag", () => {
  it("Environment › Rename renames the tag for the group", async () => {
    mocked.renameEnvironment.mockResolvedValue({ status: "ok", data: null })
    renderMenu()
    fireEvent.click(await screen.findByText("Environment"))
    fireEvent.click(await screen.findByText("Rename Local…"))
    const input = await screen.findByLabelText("Environment name")
    fireEvent.change(input, { target: { value: "Dev" } })
    fireEvent.click(screen.getByRole("button", { name: "Rename" }))
    await waitFor(() =>
      expect(mocked.renameEnvironment).toHaveBeenCalledWith({
        environmentId: "e1",
        newLabel: "Dev",
      }),
    )
  })
})
