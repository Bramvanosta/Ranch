import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { renameGroup: vi.fn(), deleteGroup: vi.fn() },
}))
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }))
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  // Nothing is open: the selected connection comes from the URL.
  useRouterState: ({ select }: { select: (s: { location: { pathname: string } }) => unknown }) =>
    select({ location: { pathname: "/" } }),
}))
// The "New environment" dialog has its own tests; keep this suite on the header.
vi.mock("@/features/connections/NewPlacementDialog", () => ({
  NewPlacementDialog: ({ open, group }: { open: boolean; group?: { name: string } }) =>
    open ? <div data-testid="new-env-dialog">{group?.name}</div> : null,
}))

import { Collapsible } from "@/components/ui/collapsible"
import { type Connection, commands } from "@/lib/tauri/bindings"

import { useSidebarUiStore } from "@/stores/useSidebarUiStore"

import { GroupHeader } from "./GroupHeader"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>

const connection = {
  id: "c1",
  name: "lumen-redis",
  envLabel: "Local",
  envColor: "env-local",
} as Connection

const allGroups = [
  { id: "g1", name: "Lumen" },
  { id: "g2", name: "Acme" },
]

afterEach(() => vi.resetAllMocks())

function renderHeader(connections: Connection[] = []) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <Collapsible open>
        <GroupHeader
          groupId="g1"
          name="Lumen"
          open
          connections={connections}
          allGroups={allGroups}
        />
      </Collapsible>
    </QueryClientProvider>,
  )
}

describe("GroupHeader", () => {
  it("shows the name and the connection count", () => {
    renderHeader([connection])
    expect(screen.getByText("Lumen")).toBeInTheDocument()
    expect(screen.getByTestId("group-header-g1")).toHaveTextContent("1")
  })

  it("F2 on the chevron starts an inline rename; Enter commits the trimmed name", async () => {
    mocked.renameGroup.mockResolvedValue({ status: "ok", data: {} })
    renderHeader()
    fireEvent.keyDown(screen.getByRole("button", { name: "Toggle Lumen" }), { key: "F2" })
    const input = screen.getByRole("textbox", { name: "Rename group Lumen" })
    fireEvent.change(input, { target: { value: "  Lumen SaaS " } })
    fireEvent.keyDown(input, { key: "Enter" })
    await waitFor(() =>
      expect(mocked.renameGroup).toHaveBeenCalledWith({ id: "g1", name: "Lumen SaaS" }),
    )
  })

  it("double-click renames; a duplicate name is refused inline without a backend call", () => {
    renderHeader()
    fireEvent.doubleClick(screen.getByText("Lumen"))
    const input = screen.getByRole("textbox", { name: "Rename group Lumen" })
    fireEvent.change(input, { target: { value: "acme" } })
    fireEvent.keyDown(input, { key: "Enter" })
    expect(screen.getByRole("alert")).toHaveTextContent(/already exists/)
    expect(mocked.renameGroup).not.toHaveBeenCalled()
  })

  it("Escape cancels the rename", () => {
    renderHeader()
    fireEvent.doubleClick(screen.getByText("Lumen"))
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Escape" })
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument()
    expect(mocked.renameGroup).not.toHaveBeenCalled()
  })

  it("deletes an empty group immediately, without a confirmation", async () => {
    mocked.deleteGroup.mockResolvedValue({ status: "ok", data: null })
    renderHeader([])
    fireEvent.contextMenu(screen.getByTestId("group-header-g1"))
    fireEvent.click(await screen.findByText("Delete"))
    await waitFor(() =>
      expect(mocked.deleteGroup).toHaveBeenCalledWith({ id: "g1", cascade: false }),
    )
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument()
  })

  it("asks for confirmation listing the connections before a cascade delete", async () => {
    mocked.deleteGroup.mockResolvedValue({ status: "ok", data: null })
    renderHeader([connection])
    fireEvent.contextMenu(screen.getByTestId("group-header-g1"))
    fireEvent.click(await screen.findByText("Delete…"))

    // A destructive sheet (was an alert dialog).
    const dialog = await screen.findByRole("dialog")
    expect(dialog).toHaveTextContent("Delete Lumen?")
    expect(dialog).toHaveTextContent("lumen-redis")
    expect(mocked.deleteGroup).not.toHaveBeenCalled()

    fireEvent.click(screen.getByRole("button", { name: "Delete group" }))
    await waitFor(() =>
      expect(mocked.deleteGroup).toHaveBeenCalledWith({ id: "g1", cascade: true }),
    )
  })

  // ── Group menu: new connection, new environment, rename ───────────────

  it('"New connection here" asks the sidebar to open the form on this group', async () => {
    renderHeader()
    fireEvent.contextMenu(screen.getByTestId("group-header-g1"))
    fireEvent.click(await screen.findByText("New connection here"))
    expect(useSidebarUiStore.getState().addConnectionTarget).toEqual({ groupId: "g1" })
    useSidebarUiStore.getState().closeAddConnection()
  })

  it('"New environment" opens the dialog for this group', async () => {
    renderHeader()
    fireEvent.contextMenu(screen.getByTestId("group-header-g1"))
    fireEvent.click(await screen.findByText("New environment"))
    expect(await screen.findByTestId("new-env-dialog")).toHaveTextContent("Lumen")
  })

  it('a palette "Rename group" intent starts the inline rename and is cleared', async () => {
    renderHeader()
    useSidebarUiStore.getState().revealGroup("g1", { rename: true })
    expect(await screen.findByRole("textbox", { name: "Rename group Lumen" })).toBeInTheDocument()
    expect(useSidebarUiStore.getState().revealGroupId).toBeNull()
  })
})
