import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { removeConnection: vi.fn() },
}))
vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn() },
}))
const navigateMock = vi.fn()
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigateMock,
}))

import { toast } from "sonner"
import { commands } from "@/lib/tauri/bindings"
import { useSelectionStore } from "@/stores/useSelectionStore"

import { RemoveConnectionAlertDialog } from "./RemoveConnectionAlertDialog"

const mocked = commands as unknown as { removeConnection: ReturnType<typeof vi.fn> }
const mockedToast = toast as unknown as {
  error: ReturnType<typeof vi.fn>
  success: ReturnType<typeof vi.fn>
}

const fakeConnection = {
  id: "conn-1",
  environmentId: "e1",
  groupId: "g1",
  name: "My Redis",
  host: "h",
  port: 6379,
  db: 0,
  username: null,
  tls: false,
  envColor: "env-local",
  envName: "Local",
  envLabel: "Environment",
  groupName: "Default",
  createdAt: "2026-05-03T00:00:00Z",
  mcpReadonly: null,
  sidebarCollapsed: false,
  tlsCustomCa: null,
  sshTunnel: null,
}

afterEach(() => {
  vi.resetAllMocks()
  // Reset Zustand store between tests.
  useSelectionStore.setState({ selectedConnectionId: null })
})

function renderDialog(onOpenChange = vi.fn()) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return {
    onOpenChange,
    ...render(
      <QueryClientProvider client={client}>
        <RemoveConnectionAlertDialog connection={fakeConnection} open onOpenChange={onOpenChange} />
      </QueryClientProvider>,
    ),
  }
}

describe("RemoveConnectionAlertDialog", () => {
  it("shows the connection name in the title and what is kept", () => {
    renderDialog()
    expect(screen.getByText("Remove “My Redis”?")).toBeInTheDocument()
    expect(screen.getByText(/Nothing changes on the Redis server/)).toBeInTheDocument()
  })

  it("Cancel does not call removeConnection", () => {
    const { onOpenChange } = renderDialog()
    fireEvent.click(screen.getByRole("button", { name: /Cancel/i }))
    expect(mocked.removeConnection).not.toHaveBeenCalled()
    // Both base-ui's auto-close (passes (open, details)) AND our explicit
    // `onOpenChange(false)` from the mutation onSuccess satisfy this — we
    // assert on the first arg only.
    expect(onOpenChange).toHaveBeenCalled()
    expect(onOpenChange.mock.calls[0][0]).toBe(false)
  })

  it("Confirm calls removeConnection and closes on success", async () => {
    mocked.removeConnection.mockResolvedValue({ status: "ok", data: null })
    const { onOpenChange } = renderDialog()

    fireEvent.click(screen.getByRole("button", { name: "Remove Connection" }))

    await waitFor(() => {
      expect(mocked.removeConnection).toHaveBeenCalledWith("conn-1")
    })
    await waitFor(() => {
      expect(mockedToast.success).toHaveBeenCalledWith("My Redis removed")
    })
    // Both base-ui's auto-close (passes (open, details)) AND our explicit
    // `onOpenChange(false)` from the mutation onSuccess satisfy this — we
    // assert on the first arg only.
    expect(onOpenChange).toHaveBeenCalled()
    expect(onOpenChange.mock.calls[0][0]).toBe(false)
  })

  it("navigates to / when removing the currently-selected connection", async () => {
    useSelectionStore.setState({ selectedConnectionId: "conn-1" })
    mocked.removeConnection.mockResolvedValue({ status: "ok", data: null })
    renderDialog()

    fireEvent.click(screen.getByRole("button", { name: "Remove Connection" }))

    await waitFor(() => {
      expect(navigateMock).toHaveBeenCalledWith({ to: "/" })
    })
  })

  it("does NOT navigate when removing a non-selected connection", async () => {
    useSelectionStore.setState({ selectedConnectionId: "different-id" })
    mocked.removeConnection.mockResolvedValue({ status: "ok", data: null })
    renderDialog()

    fireEvent.click(screen.getByRole("button", { name: "Remove Connection" }))

    await waitFor(() => {
      expect(mocked.removeConnection).toHaveBeenCalled()
    })
    expect(navigateMock).not.toHaveBeenCalled()
  })

  it("shows an error toast when the backend fails", async () => {
    mocked.removeConnection.mockResolvedValue({
      status: "error",
      error: { kind: "keychainUnavailable", message: "no service", docUrl: null },
    })
    renderDialog()

    fireEvent.click(screen.getByRole("button", { name: "Remove Connection" }))

    await waitFor(() => {
      expect(mockedToast.error).toHaveBeenCalledWith(expect.stringContaining("Could not remove"))
    })
  })
})
