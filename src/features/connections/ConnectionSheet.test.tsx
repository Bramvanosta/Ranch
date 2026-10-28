import { fireEvent, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

// Mock the generated bindings before importing the component.
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    parseRedisUrl: vi.fn(),
    testConnection: vi.fn(),
    addConnection: vi.fn(),
    updateConnection: vi.fn(),
    listConnections: vi.fn(),
    listBackends: vi.fn(),
    setConnectionMcpReadonly: vi.fn(),
  },
}))

// Placement reads groups + environments through these hooks.
// Default: nothing exists yet, so the sheet proposes a new "Default" / "Local".
const placementData = vi.hoisted(() => ({
  groups: [] as unknown[],
  environments: [] as unknown[],
  lockState: undefined as "locked" | "unlocked" | undefined,
}))
vi.mock("@/data/groups/queries", () => ({
  useGroups: () => ({ data: placementData.groups, isSuccess: true }),
}))
vi.mock("@/data/environments/queries", () => ({
  useEnvironments: () => ({ data: placementData.environments, isSuccess: true }),
}))
vi.mock("@/data/mcp/queries", async (orig) => ({
  ...(await orig<object>()),
  useMcpSafetySettings: () => ({ data: { readonlyDefault: false } }),
}))
vi.mock("@/data/safety/queries", () => ({
  useConnectionLockState: () => ({ data: placementData.lockState }),
}))
vi.mock("@/lib/runtime/runtime", () => ({ hasAgents: () => true }))

import { BULLMQ_CAPABILITIES } from "@/backend/fake/backends"
import { bullmqConnection, redisEndpoint } from "@/domain/bullmqEndpoint"
import { type BackendInfo, type Connection, commands } from "@/lib/tauri/bindings"
import { renderWithProviders, testQueryClient } from "@/test/renderWithProviders"
import { ConnectionSheet } from "./ConnectionSheet"

const mocked = commands as unknown as Record<
  | "parseRedisUrl"
  | "testConnection"
  | "addConnection"
  | "updateConnection"
  | "listBackends"
  | "setConnectionMcpReadonly",
  ReturnType<typeof vi.fn>
>

afterEach(() => {
  vi.resetAllMocks()
  placementData.groups = []
  placementData.environments = []
  placementData.lockState = undefined
})

const PROBE = {
  latencyMs: 2,
  queuesFound: 7,
  queueSample: ["bull:emails", "bull:exports"],
  scanCapped: false,
  schedulers: 3,
  workers: 4,
}
const succeeded = (extra: Record<string, unknown> = {}) => ({
  status: "ok",
  data: {
    success: true,
    redisVersion: "7.4.2",
    error: null,
    sshHostKeyPinned: null,
    probe: PROBE,
    ...extra,
  },
})
const failedWith = (error: unknown) => ({
  status: "ok",
  data: { success: false, redisVersion: null, error, sshHostKeyPinned: null },
})

function openSheet(props: Record<string, unknown> = {}, backends?: BackendInfo[]) {
  const onSaved = vi.fn()
  const all = { open: true, onOpenChange: vi.fn(), onSaved, ...props }
  renderWithProviders(
    <ConnectionSheet {...(all as unknown as Parameters<typeof ConnectionSheet>[0])} />,
    { client: testQueryClient(backends) },
  )
  return { onSaved }
}

function fillHost(host = "h") {
  fireEvent.change(screen.getByLabelText(/^Host$/i), { target: { value: host } })
}

async function runTest() {
  fireEvent.click(screen.getByRole("button", { name: /Test connection/i }))
  return screen.findByTestId("test-result")
}

function addButton() {
  return screen.getByRole("button", { name: /Add connection/i })
}

describe("ConnectionSheet: step 1, the server", () => {
  it("pasting a URL fills host, port, database, user, password and TLS", async () => {
    mocked.parseRedisUrl.mockResolvedValue({
      status: "ok",
      data: {
        host: "redis.example.com",
        port: 6380,
        db: 2,
        username: "alice",
        password: "secret",
        tls: true,
      },
    })
    openSheet()
    fireEvent.change(screen.getByLabelText(/Paste a Redis URL/i), {
      target: { value: "rediss://alice:secret@redis.example.com:6380/2" },
    })
    await waitFor(() =>
      expect((screen.getByLabelText(/^Host$/i) as HTMLInputElement).value).toBe(
        "redis.example.com",
      ),
    )
    expect((screen.getByLabelText(/^Port$/i) as HTMLInputElement).value).toBe("6380")
    expect((screen.getByLabelText(/^DB index$/i) as HTMLInputElement).value).toBe("2")
    expect((screen.getByLabelText(/Username/i) as HTMLInputElement).value).toBe("alice")
    expect((screen.getByLabelText(/^Password \(optional\)$/i) as HTMLInputElement).value).toBe(
      "secret",
    )
    expect((screen.getByLabelText(/Use TLS/i) as HTMLInputElement).checked).toBe(true)
  })

  it("an invalid URL says so inline", async () => {
    mocked.parseRedisUrl.mockResolvedValue({
      status: "error",
      error: { kind: "invalidInput", message: "Not a Redis URL" },
    })
    openSheet()
    fireEvent.change(screen.getByLabelText(/Paste a Redis URL/i), { target: { value: "http://x" } })
    expect(await screen.findByText("Not a Redis URL")).toBeInTheDocument()
  })

  it("TLS and the SSH tunnel are folded sections, open when in use", () => {
    openSheet()
    expect(screen.getByTestId("tls-disclosure")).not.toHaveAttribute("open")
    expect(screen.getByTestId("ssh-disclosure")).not.toHaveAttribute("open")
    expect(screen.queryByTestId("advanced-tls-section")).not.toBeInTheDocument()
    expect(screen.queryByTestId("advanced-ssh-section")).not.toBeInTheDocument()
  })

  it("asks for nothing else than the server: no name, no placement yet", () => {
    openSheet()
    expect(screen.queryByLabelText(/Connection name/i)).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: /Add connection/i })).not.toBeInTheDocument()
  })

  it("Test connection waits for a host", () => {
    openSheet()
    expect(screen.getByRole("button", { name: /Test connection/i })).toBeDisabled()
    fillHost()
    expect(screen.getByRole("button", { name: /Test connection/i })).not.toBeDisabled()
  })
})

describe("ConnectionSheet: step 2, what the test found", () => {
  it("shows latency, version, TLS, queues with a sample, schedulers and workers", async () => {
    mocked.testConnection.mockResolvedValue(succeeded())
    openSheet()
    fillHost()
    await runTest()
    const facts = screen.getByTestId("probe-facts")
    expect(facts).toHaveTextContent("2 ms")
    expect(facts).toHaveTextContent("v7.4.2")
    expect(facts).toHaveTextContent(/TLSoff/)
    expect(facts).toHaveTextContent("bull:emails, bull:exports and 5 more")
    expect(facts).toHaveTextContent(/Schedulers3/)
    expect(facts).toHaveTextContent(/Workers4/)
    expect(addButton()).not.toBeDisabled()
  })

  it("starts the name from the host, then saves with addConnection", async () => {
    mocked.testConnection.mockResolvedValue(succeeded())
    mocked.addConnection.mockResolvedValue({ status: "ok", data: { id: "01928a3d-ulid" } })
    const { onSaved } = openSheet()
    fillHost("redis.local")
    await runTest()
    const name = screen.getByLabelText(/Connection name/i) as HTMLInputElement
    expect(name.value).toBe("redis.local")
    fireEvent.change(name, { target: { value: "test" } })
    fireEvent.click(addButton())
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith("01928a3d-ulid"))
    expect(mocked.addConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "test",
        backend: "bullmq-redis",
        endpoint: redisEndpoint({ host: "redis.local" }),
        locked: null,
        group: { kind: "new", name: "Default" },
        environment: { kind: "new", name: "Local", colorToken: "env-local" },
      }),
    )
  })

  it("a failed test explains itself in plain words, and Back keeps the values", async () => {
    mocked.testConnection.mockResolvedValue(failedWith({ kind: "authFailed" }))
    openSheet()
    fillHost("redis.local")
    fireEvent.change(screen.getByLabelText(/^Password \(optional\)$/i), {
      target: { value: "wrong" },
    })
    await runTest()
    expect(screen.getByText(/Redis refused the user or password/i)).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: /Add connection/i })).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole("button", { name: /^Back$/i }))
    expect((screen.getByLabelText(/^Host$/i) as HTMLInputElement).value).toBe("redis.local")
    expect((screen.getByLabelText(/^Password \(optional\)$/i) as HTMLInputElement).value).toBe(
      "wrong",
    )
  })

  it("a refused connection says what to check, with the raw message under it", async () => {
    mocked.testConnection.mockResolvedValue(
      failedWith({ kind: "cannotReach", reason: "Connection refused (os error 61)" }),
    )
    openSheet()
    fillHost()
    await runTest()
    expect(screen.getByText("Connection refused. Is Redis running on port 6379?")).toBeVisible()
    expect(screen.getByText("Connection refused (os error 61)")).toBeInTheDocument()
  })

  it("Try again runs the test once more", async () => {
    mocked.testConnection.mockResolvedValueOnce(failedWith({ kind: "timeout" }))
    mocked.testConnection.mockResolvedValueOnce(succeeded())
    openSheet()
    fillHost()
    await runTest()
    expect(screen.getByText(/No answer from h:6379 within 4 seconds/i)).toBeInTheDocument()
    fireEvent.click(screen.getByRole("button", { name: /Try again/i }))
    await waitFor(() => expect(screen.getByTestId("probe-facts")).toBeInTheDocument())
    expect(mocked.testConnection).toHaveBeenCalledTimes(2)
  })

  it("renders the keychain doc link when add fails with keychainUnavailable", async () => {
    mocked.testConnection.mockResolvedValue(succeeded())
    mocked.addConnection.mockResolvedValue({
      status: "error",
      error: {
        kind: "keychainUnavailable",
        message: "secret service not running",
        docUrl: "https://example.test/keychain",
      },
    })
    openSheet()
    fillHost()
    await runTest()
    fireEvent.click(addButton())
    expect(await screen.findByText(/secret service not running/i)).toBeInTheDocument()
    expect(screen.getByRole("link", { name: /How to set up the keychain/i })).toHaveAttribute(
      "href",
      "https://example.test/keychain",
    )
  })
})

describe("ConnectionSheet: read-only until unlocked", () => {
  it("is on for a new Production environment, and an untouched switch sends null", async () => {
    mocked.testConnection.mockResolvedValue(succeeded())
    openSheet()
    fillHost()
    await runTest()
    const lock = screen.getByRole("switch", { name: /Read-only until I unlock it/i })
    expect(lock).not.toBeChecked()
    fireEvent.click(screen.getByRole("radio", { name: /Production/i }))
    expect(lock).toBeChecked()
    expect(screen.getByText(/On by default for Production/i)).toBeInTheDocument()
  })

  it("a touched switch is sent as the lock choice", async () => {
    mocked.testConnection.mockResolvedValue(succeeded())
    mocked.addConnection.mockResolvedValue({ status: "ok", data: { id: "n" } })
    openSheet()
    fillHost()
    await runTest()
    fireEvent.click(screen.getByRole("switch", { name: /Read-only until I unlock it/i }))
    fireEvent.click(addButton())
    await waitFor(() =>
      expect(mocked.addConnection).toHaveBeenCalledWith(expect.objectContaining({ locked: true })),
    )
  })
})

describe("ConnectionSheet: TLS", () => {
  it("reveals the CA field when TLS turns on, and keeps the PEM across off and on", () => {
    openSheet()
    fireEvent.click(screen.getByLabelText(/Use TLS/i))
    const textarea = screen.getByLabelText(/Custom CA certificate/i) as HTMLTextAreaElement
    fireEvent.change(textarea, {
      target: { value: "-----BEGIN CERTIFICATE-----\nXYZ\n-----END CERTIFICATE-----" },
    })
    fireEvent.click(screen.getByLabelText(/Use TLS/i))
    expect(screen.queryByTestId("advanced-tls-section")).not.toBeInTheDocument()
    fireEvent.click(screen.getByLabelText(/Use TLS/i))
    expect(
      (screen.getByLabelText(/Custom CA certificate/i) as HTMLTextAreaElement).value,
    ).toContain("XYZ")
  })

  it("Clear empties the CA field and disables itself", () => {
    openSheet()
    fireEvent.click(screen.getByLabelText(/Use TLS/i))
    const textarea = screen.getByLabelText(/Custom CA certificate/i) as HTMLTextAreaElement
    fireEvent.change(textarea, { target: { value: "-----BEGIN CERTIFICATE-----" } })
    const clear = screen.getByRole("button", { name: /Clear/i })
    fireEvent.click(clear)
    expect(textarea.value).toBe("")
    expect(clear).toBeDisabled()
  })

  it("sends the PEM only with TLS on", async () => {
    mocked.testConnection.mockResolvedValue(succeeded())
    mocked.addConnection.mockResolvedValue({ status: "ok", data: { id: "id1" } })
    openSheet()
    fillHost()
    fireEvent.click(screen.getByLabelText(/Use TLS/i))
    fireEvent.change(screen.getByLabelText(/Custom CA certificate/i), {
      target: { value: "-----BEGIN CERTIFICATE-----\nA\n-----END CERTIFICATE-----" },
    })
    await runTest()
    expect(mocked.testConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        endpoint: expect.objectContaining({
          tls: true,
          tlsCustomCa: expect.stringContaining("BEGIN"),
        }),
      }),
    )
    fireEvent.click(addButton())
    await waitFor(() => expect(mocked.addConnection).toHaveBeenCalled())
    expect(mocked.addConnection.mock.calls[0][0].endpoint.tlsCustomCa).toContain(
      "BEGIN CERTIFICATE",
    )
  })

  it("sends tlsCustomCa: null once TLS is off again", async () => {
    mocked.testConnection.mockResolvedValue(succeeded())
    mocked.addConnection.mockResolvedValue({ status: "ok", data: { id: "id2" } })
    openSheet()
    fillHost()
    fireEvent.click(screen.getByLabelText(/Use TLS/i))
    fireEvent.change(screen.getByLabelText(/Custom CA certificate/i), {
      target: { value: "-----BEGIN CERTIFICATE-----" },
    })
    fireEvent.click(screen.getByLabelText(/Use TLS/i))
    await runTest()
    fireEvent.click(addButton())
    await waitFor(() => expect(mocked.addConnection).toHaveBeenCalled())
    expect(mocked.addConnection.mock.calls[0][0].endpoint).toMatchObject({
      tls: false,
      tlsCustomCa: null,
    })
  })
})

describe("ConnectionSheet: SSH tunnel", () => {
  function sshOn() {
    fireEvent.click(screen.getByLabelText(/Use SSH tunnel/i))
    fireEvent.change(screen.getByLabelText(/^SSH host$/i), { target: { value: "bastion" } })
    fireEvent.change(screen.getByLabelText(/^SSH user$/i), { target: { value: "ubuntu" } })
  }

  it("switches between key file and password, and reveals the jump host", () => {
    openSheet()
    fireEvent.click(screen.getByLabelText(/Use SSH tunnel/i))
    expect(screen.getByLabelText(/SSH key file path/i)).toBeInTheDocument()
    fireEvent.click(screen.getByLabelText(/^Password$/i))
    expect(screen.queryByLabelText(/SSH key file path/i)).not.toBeInTheDocument()
    expect(screen.getByLabelText(/^SSH password$/i)).toBeInTheDocument()
    expect(screen.queryByLabelText(/^Jump host$/i)).not.toBeInTheDocument()
    fireEvent.click(screen.getByLabelText(/Use jump host/i))
    expect(screen.getByLabelText(/^Jump host$/i)).toBeInTheDocument()
    expect(screen.getByLabelText(/^Jump port$/i)).toBeInTheDocument()
    expect(screen.getByLabelText(/^Jump user$/i)).toBeInTheDocument()
  })

  it("a tunnel that cannot open is explained, with the bastion's own words", async () => {
    mocked.testConnection.mockResolvedValue(
      failedWith({ kind: "sshTunnel", reason: "Bastion rejected public-key authentication" }),
    )
    openSheet()
    fillHost()
    sshOn()
    await runTest()
    expect(screen.getByText(/The SSH tunnel could not be opened/i)).toBeInTheDocument()
    expect(screen.getByText("Bastion rejected public-key authentication")).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: /Add connection/i })).not.toBeInTheDocument()
  })

  it("shows the full host key the first connect pinned", async () => {
    mocked.testConnection.mockResolvedValue(
      succeeded({
        sshHostKeyPinned: {
          fingerprint: "SHA256:abcdefghijkl1234567890abcdef0123456789ABC",
          wasNewPin: true,
        },
      }),
    )
    openSheet()
    fillHost()
    sshOn()
    await runTest()
    expect(screen.getByTestId("ssh-hostkey-pinned-banner")).toHaveTextContent(
      "SHA256:abcdefghijkl1234567890abcdef0123456789ABC",
    )
  })

  it("sends the tunnel and its password on test and on save", async () => {
    mocked.testConnection.mockResolvedValue(succeeded())
    mocked.addConnection.mockResolvedValue({ status: "ok", data: { id: "pw" } })
    openSheet()
    fillHost()
    sshOn()
    fireEvent.click(screen.getByLabelText(/^Password$/i))
    fireEvent.change(screen.getByLabelText(/^SSH password$/i), { target: { value: "ssh-secret" } })
    await runTest()
    expect(mocked.testConnection).toHaveBeenCalledWith(
      expect.objectContaining({ sshPassword: "ssh-secret", savedConnectionId: null }),
    )
    fireEvent.click(addButton())
    await waitFor(() => expect(mocked.addConnection).toHaveBeenCalled())
    const payload = mocked.addConnection.mock.calls[0][0]
    expect(payload.endpoint.sshTunnel).toMatchObject({ user: "ubuntu", auth: { kind: "password" } })
    expect(payload.sshPassword).toBe("ssh-secret")
  })

  it("sends no tunnel once SSH is off again", async () => {
    mocked.testConnection.mockResolvedValue(succeeded())
    mocked.addConnection.mockResolvedValue({ status: "ok", data: { id: "n" } })
    openSheet()
    fillHost()
    sshOn()
    fireEvent.click(screen.getByLabelText(/Use SSH tunnel/i))
    await runTest()
    fireEvent.click(addButton())
    await waitFor(() => expect(mocked.addConnection).toHaveBeenCalled())
    expect(mocked.addConnection.mock.calls[0][0]).toMatchObject({
      endpoint: { sshTunnel: null },
      sshPassword: null,
    })
  })
})

describe("ConnectionSheet: placement", () => {
  const groups = [
    { id: "g1", name: "Lumen", displayOrder: 0, collapsed: false, connectionCount: 1 },
    { id: "g2", name: "Acme", displayOrder: 1, collapsed: false, connectionCount: 0 },
  ]
  const environments = [
    { id: "e1", groupId: "g1", label: "Local", name: "Local", colorToken: "env-local" },
    {
      id: "e2",
      groupId: "g1",
      label: "Production",
      name: "Production",
      colorToken: "env-production",
    },
  ]

  async function reachPlacement(props: Record<string, unknown> = {}) {
    placementData.groups = groups
    placementData.environments = environments
    mocked.testConnection.mockResolvedValue(succeeded())
    mocked.addConnection.mockResolvedValue({ status: "ok", data: { id: "new-id" } })
    openSheet(props)
    fillHost()
    await runTest()
  }

  it("preselects the first group and its first environment, and sends their ids", async () => {
    await reachPlacement()
    await waitFor(() =>
      expect((screen.getByLabelText("Group") as HTMLSelectElement).value).toBe("g1"),
    )
    expect((screen.getByLabelText("Environment") as HTMLSelectElement).value).toBe("e1")
    expect(screen.getByTestId("existing-env-color")).toBeInTheDocument()
    fireEvent.click(addButton())
    await waitFor(() =>
      expect(mocked.addConnection).toHaveBeenCalledWith(
        expect.objectContaining({
          group: { kind: "existing", id: "g1" },
          environment: { kind: "existing", id: "e1" },
        }),
      ),
    )
  })

  it('honours preferredPlacement ("New connection here")', async () => {
    await reachPlacement({ preferredPlacement: { groupId: "g1", environmentId: "e2" } })
    await waitFor(() =>
      expect((screen.getByLabelText("Environment") as HTMLSelectElement).value).toBe("e2"),
    )
    // Placed in Production: read-only until unlocked, before any touch.
    expect(screen.getByRole("switch", { name: /Read-only until I unlock it/i })).toBeChecked()
  })

  it("an empty group only offers a new environment, named after the chosen color", async () => {
    await reachPlacement()
    await waitFor(() =>
      expect((screen.getByLabelText("Group") as HTMLSelectElement).value).toBe("g1"),
    )
    fireEvent.change(screen.getByLabelText("Group"), { target: { value: "g2" } })
    const envName = screen.getByLabelText("New environment name") as HTMLInputElement
    expect(envName.value).toBe("Local")
    fireEvent.click(screen.getByRole("radio", { name: /Staging/i }))
    expect(envName.value).toBe("Staging")
    fireEvent.change(envName, { target: { value: "EU West" } })
    fireEvent.click(screen.getByRole("radio", { name: /Production/i }))
    expect(envName.value).toBe("EU West")
    fireEvent.click(addButton())
    await waitFor(() =>
      expect(mocked.addConnection).toHaveBeenCalledWith(
        expect.objectContaining({
          group: { kind: "existing", id: "g2" },
          environment: { kind: "new", name: "EU West", colorToken: "env-production" },
        }),
      ),
    )
  })

  it("a new group requires a name and forces a new environment", async () => {
    await reachPlacement()
    await waitFor(() =>
      expect((screen.getByLabelText("Group") as HTMLSelectElement).value).toBe("g1"),
    )
    fireEvent.change(screen.getByLabelText("Group"), { target: { value: "__new__" } })
    expect(screen.getByLabelText("Environment")).toBeDisabled()
    fireEvent.click(addButton())
    expect(await screen.findByText("Give the group a name")).toBeInTheDocument()
    expect(mocked.addConnection).not.toHaveBeenCalled()
  })
})

describe("ConnectionSheet: agent read-only switch", () => {
  async function reachStepTwo() {
    mocked.testConnection.mockResolvedValue(succeeded())
    mocked.addConnection.mockResolvedValue({ status: "ok", data: { id: "new-id" } })
    const opened = openSheet()
    fillHost()
    await runTest()
    return opened
  }

  it("an untouched switch sends nothing: the connection keeps following the default", async () => {
    const { onSaved } = await reachStepTwo()
    fireEvent.click(addButton())
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith("new-id"))
    expect(mocked.setConnectionMcpReadonly).not.toHaveBeenCalled()
  })

  it("a touched switch is saved for the new connection, after the connection itself", async () => {
    mocked.setConnectionMcpReadonly.mockResolvedValue({ status: "ok", data: null })
    const { onSaved } = await reachStepTwo()
    fireEvent.click(screen.getByRole("switch", { name: /Agents \(MCP\) can only read/i }))
    fireEvent.click(addButton())
    await waitFor(() =>
      expect(mocked.setConnectionMcpReadonly).toHaveBeenCalledWith({
        connectionId: "new-id",
        mcpReadonly: true,
      }),
    )
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith("new-id"))
  })

  it("shows ON for a new Production environment before the user touches anything", async () => {
    await reachStepTwo()
    const toggle = screen.getByRole("switch", { name: /Agents \(MCP\) can only read/i })
    expect(toggle).not.toBeChecked()
    fireEvent.click(screen.getByRole("radio", { name: /Production/i }))
    expect(toggle).toBeChecked()
  })
})

describe("ConnectionSheet: editing", () => {
  const connection = {
    id: "c1",
    name: "Prod",
    ...bullmqConnection(
      redisEndpoint({ host: "redis.prod", port: 6380, db: 1, username: "app", tls: true }),
    ),
    groupId: "g1",
    environmentId: "e2",
    envColor: "env-production",
    mcpReadonly: null,
  } as unknown as Connection

  it("uses the same two steps with the values filled, the password blank", async () => {
    placementData.lockState = "locked"
    mocked.testConnection.mockResolvedValue(succeeded())
    mocked.updateConnection.mockResolvedValue({ status: "ok", data: { id: "c1" } })
    const { onSaved } = openSheet({ mode: "edit", connection })
    expect((screen.getByLabelText(/^Host$/i) as HTMLInputElement).value).toBe("redis.prod")
    const password = screen.getByLabelText(/^Password \(optional\)$/i) as HTMLInputElement
    expect(password.value).toBe("")
    expect(password.placeholder).toBe("Leave blank to keep current")

    await runTest()
    // A blank password tests with the saved one.
    expect(mocked.testConnection).toHaveBeenCalledWith(
      expect.objectContaining({ password: null, savedConnectionId: "c1" }),
    )
    expect((screen.getByLabelText(/Connection name/i) as HTMLInputElement).value).toBe("Prod")
    expect(screen.getByRole("switch", { name: /Read-only until I unlock it/i })).toBeChecked()
    fireEvent.click(screen.getByRole("button", { name: /Save changes/i }))
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith("c1"))
    // The whole endpoint goes back, the password beside it.
    expect(mocked.updateConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "c1",
        backend: "bullmq-redis",
        endpoint: redisEndpoint({
          host: "redis.prod",
          port: 6380,
          db: 1,
          username: "app",
          tls: true,
        }),
        password: null,
        locked: null,
      }),
    )
  })

  it("turning the tunnel off sends an endpoint without one", async () => {
    mocked.testConnection.mockResolvedValue(succeeded())
    mocked.updateConnection.mockResolvedValue({ status: "ok", data: { id: "c1" } })
    const tunnelled = {
      ...connection,
      ...bullmqConnection(
        redisEndpoint({
          host: "redis.prod",
          sshTunnel: {
            host: "bastion",
            port: 22,
            user: "ops",
            auth: { kind: "password" },
            jumpHost: null,
          },
        }),
      ),
    } as Connection
    const { onSaved } = openSheet({ mode: "edit", connection: tunnelled })
    fireEvent.click(screen.getByLabelText(/Use SSH tunnel/i))
    await runTest()
    fireEvent.click(screen.getByRole("button", { name: /Save changes/i }))
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith("c1"))
    expect(mocked.updateConnection.mock.calls[0][0]).toMatchObject({
      endpoint: { host: "redis.prod", sshTunnel: null },
      sshPassword: null,
    })
  })

  it("names the backend it has no form for", () => {
    openSheet({ mode: "edit", connection: { ...connection, backend: "memory", endpoint: {} } })
    expect(screen.getByText(/Ranch has no form for a memory connection/i)).toBeInTheDocument()
    expect(screen.queryByLabelText(/^Host$/i)).not.toBeInTheDocument()
  })
})

describe("ConnectionSheet: the backend's endpoint schema", () => {
  it("labels the fields as the backend lists them", async () => {
    openSheet({}, [
      {
        kind: "bullmq-redis",
        label: "BullMQ on Redis",
        endpointSchema: {
          fields: [
            { key: "host", label: "Server", kind: "text", required: true, default: null },
            { key: "username", label: "User", kind: "text", required: false, default: null },
          ],
        },
        capabilities: BULLMQ_CAPABILITIES,
      },
    ])
    expect(await screen.findByLabelText(/^Server$/i)).toBeInTheDocument()
    expect(screen.getByLabelText(/^User \(optional\)$/i)).toBeInTheDocument()
    // A field the schema leaves out keeps its own label.
    expect(screen.getByLabelText(/^Port$/i)).toBeInTheDocument()
  })
})
