import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { bullmqConnection, redisEndpoint } from "@/domain/bullmqEndpoint"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    parseRedisUrl: vi.fn(),
    testConnection: vi.fn(),
    updateConnection: vi.fn(),
    addConnection: vi.fn(),
  },
}))
vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn() },
}))
// The saved connection's group + environment exist.
vi.mock("@/data/groups/queries", () => ({
  useGroups: () => ({
    data: [{ id: "g1", name: "Default", displayOrder: 0, collapsed: false, connectionCount: 1 }],
    isSuccess: true,
  }),
}))
vi.mock("@/data/mcp/queries", async (orig) => ({
  ...(await orig<object>()),
  useMcpSafetySettings: () => ({ data: { readonlyDefault: false } }),
}))
vi.mock("@/data/safety/queries", async (orig) => ({
  ...(await orig<object>()),
  useConnectionLockState: () => ({ data: "unlocked" }),
}))
vi.mock("@/data/environments/queries", () => ({
  useEnvironments: () => ({
    data: [
      { id: "e1", groupId: "g1", label: "Staging", name: "Staging", colorToken: "env-staging" },
    ],
    isSuccess: true,
  }),
}))

import { commands } from "@/lib/tauri/bindings"

import { EditConnectionDialog } from "./EditConnectionDialog"

const mocked = commands as unknown as {
  parseRedisUrl: ReturnType<typeof vi.fn>
  testConnection: ReturnType<typeof vi.fn>
  updateConnection: ReturnType<typeof vi.fn>
}

const fakeConnection = {
  id: "conn-42",
  environmentId: "e1",
  groupId: "g1",
  name: "My Redis",
  ...bullmqConnection(
    redisEndpoint({ host: "redis.example.com", port: 6380, db: 2, username: "admin", tls: true }),
  ),
  envColor: "env-staging",
  envName: "Staging",
  envLabel: "Environment",
  groupName: "Default",
  createdAt: "2026-05-03T00:00:00Z",
  mcpReadonly: null,
  sidebarCollapsed: false,
}

afterEach(() => {
  vi.resetAllMocks()
})

function renderDialog() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <EditConnectionDialog connection={fakeConnection} open onOpenChange={vi.fn()} />
    </QueryClientProvider>,
  )
}

const tested = {
  status: "ok",
  data: { success: true, redisVersion: "7.4.2", error: null, sshHostKeyPinned: null },
}

describe("EditConnectionDialog", () => {
  it("opens on step 1 with the connection's non-secret fields", () => {
    renderDialog()
    expect(screen.getByText("Edit My Redis")).toBeInTheDocument()
    expect((screen.getByLabelText(/^Host$/i) as HTMLInputElement).value).toBe("redis.example.com")
    expect((screen.getByLabelText(/^Port$/i) as HTMLInputElement).value).toBe("6380")
    expect((screen.getByLabelText(/^DB index$/i) as HTMLInputElement).value).toBe("2")
    expect((screen.getByLabelText(/Username/i) as HTMLInputElement).value).toBe("admin")
    expect((screen.getByLabelText(/Use TLS/i) as HTMLInputElement).checked).toBe(true)
  })

  it("password field is empty with the 'leave blank to keep current' placeholder", () => {
    renderDialog()
    const password = screen.getByLabelText(/^Password \(optional\)$/i) as HTMLInputElement
    expect(password.value).toBe("")
    expect(password.placeholder).toMatch(/leave blank/i)
  })

  it("step 2 keeps the name and reads 'Save changes' (not 'Add connection')", async () => {
    mocked.testConnection.mockResolvedValue(tested)
    renderDialog()
    fireEvent.click(screen.getByRole("button", { name: /Test connection/i }))
    expect(await screen.findByRole("button", { name: /Save changes/i })).toBeInTheDocument()
    expect((screen.getByLabelText(/Connection name/i) as HTMLInputElement).value).toBe("My Redis")
    expect(screen.queryByRole("button", { name: /Add connection/i })).toBeNull()
  })

  it("on save with blank password, calls updateConnection with password: null", async () => {
    mocked.testConnection.mockResolvedValue(tested)
    mocked.updateConnection.mockResolvedValue({
      status: "ok",
      data: { ...fakeConnection },
    })

    renderDialog()

    // Force the test gate to pass.
    fireEvent.click(screen.getByRole("button", { name: /Test connection/i }))
    await waitFor(() => {
      expect(screen.getByText("v7.4.2")).toBeInTheDocument()
    })

    fireEvent.click(screen.getByRole("button", { name: /Save changes/i }))

    await waitFor(() => {
      expect(mocked.updateConnection).toHaveBeenCalledWith(
        expect.objectContaining({
          id: "conn-42",
          name: "My Redis",
          backend: "bullmq-redis",
          endpoint: expect.objectContaining({ host: "redis.example.com", port: 6380, db: 2 }),
          password: null,
          // Unchanged placement is sent as the existing ids; no move.
          group: { kind: "existing", id: "g1" },
          environment: { kind: "existing", id: "e1" },
        }),
      )
    })
  })
})
