import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    getMcpSafetySettings: vi.fn(),
    setMcpSafetySettings: vi.fn(),
    setConnectionMcpReadonly: vi.fn(),
    listConnections: vi.fn(),
  },
}))
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }))

import { commands } from "@/lib/tauri/bindings"
import { effectiveMcpReadonly } from "./effectiveMcpReadonly"
import { McpSafetySection } from "./McpSafetySection"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>

const conn = (id: string, name: string, mcpReadonly: boolean | null, envLabel = "Local") => ({
  id,
  name,
  groupId: "g1",
  groupName: "Lumen",
  envLabel,
  envColor: envLabel === "Production" ? "env-production" : "env-local",
  mcpReadonly,
})

afterEach(() => vi.resetAllMocks())

const settingsOf = (readonlyDefault: boolean) => ({
  readonlyDefault,
  approvalMode: "writes_only",
  approvalTimeoutSeconds: 120,
  approvalAlwaysOnTop: true,
  approvalNotifySound: true,
})

function renderSection(readonlyDefault = false) {
  mocked.getMcpSafetySettings.mockResolvedValue({
    status: "ok",
    data: settingsOf(readonlyDefault),
  })
  mocked.listConnections.mockResolvedValue({
    status: "ok",
    data: [conn("c1", "local-redis", null), conn("c2", "prod-redis", true, "Production")],
  })
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <McpSafetySection />
    </QueryClientProvider>,
  )
}

describe("effectiveMcpReadonly", () => {
  it("uses the connection's flag, else the global default", () => {
    expect(effectiveMcpReadonly({ mcpReadonly: null }, false)).toBe(false)
    expect(effectiveMcpReadonly({ mcpReadonly: null }, true)).toBe(true)
    expect(effectiveMcpReadonly({ mcpReadonly: false }, true)).toBe(false)
    expect(effectiveMcpReadonly({ mcpReadonly: true }, false)).toBe(true)
  })
})

describe("McpSafetySection", () => {
  it("shows each connection's effective state and marks the ones following the default", async () => {
    renderSection(false)
    const local = await screen.findByTestId("mcp-readonly-row-c1")
    const prod = screen.getByTestId("mcp-readonly-row-c2")

    expect(within(local).getByText("(default)")).toBeInTheDocument()
    expect(within(local).getByRole("switch")).not.toBeChecked()

    expect(within(prod).queryByText("(default)")).not.toBeInTheDocument()
    expect(within(prod).getByRole("switch")).toBeChecked()
  })

  it("a connection following the default reflects the global switch", async () => {
    renderSection(true)
    const local = await screen.findByTestId("mcp-readonly-row-c1")
    expect(within(local).getByRole("switch")).toBeChecked()
  })

  it("toggling a connection writes an explicit value; Use default clears it", async () => {
    mocked.setConnectionMcpReadonly.mockResolvedValue({ status: "ok", data: null })
    renderSection(false)
    const local = await screen.findByTestId("mcp-readonly-row-c1")
    fireEvent.click(within(local).getByRole("switch"))
    await waitFor(() =>
      expect(mocked.setConnectionMcpReadonly).toHaveBeenCalledWith({
        connectionId: "c1",
        mcpReadonly: true,
      }),
    )

    const prod = screen.getByTestId("mcp-readonly-row-c2")
    fireEvent.click(within(prod).getByRole("button", { name: "Use default" }))
    await waitFor(() =>
      expect(mocked.setConnectionMcpReadonly).toHaveBeenCalledWith({
        connectionId: "c2",
        mcpReadonly: null,
      }),
    )
  })

  it("the global switch saves the default", async () => {
    mocked.setMcpSafetySettings.mockResolvedValue({ status: "ok", data: settingsOf(true) })
    renderSection(false)
    fireEvent.click(await screen.findByRole("switch", { name: "Read-only by default" }))
    await waitFor(() => expect(mocked.setMcpSafetySettings).toHaveBeenCalledWith(settingsOf(true)))
  })

  // ── Approval mode ─────────────────────────────────────────────────────

  it("shows the approval mode and saves a change without touching the other settings", async () => {
    mocked.setMcpSafetySettings.mockResolvedValue({
      status: "ok",
      data: { ...settingsOf(false), approvalMode: "off" },
    })
    renderSection(false)
    const group = await screen.findByRole("radiogroup", { name: "Approval mode" })
    expect(within(group).getByRole("radio", { name: /Writes anything/ })).toBeChecked()

    fireEvent.click(within(group).getByRole("radio", { name: /Never ask/ }))
    await waitFor(() =>
      expect(mocked.setMcpSafetySettings).toHaveBeenCalledWith({
        ...settingsOf(false),
        approvalMode: "off",
      }),
    )
  })

  it("attention switches save on their own and are disabled when approval is off", async () => {
    mocked.setMcpSafetySettings.mockResolvedValue({ status: "ok", data: settingsOf(false) })
    renderSection(false)
    fireEvent.click(await screen.findByRole("switch", { name: "Notify with a sound" }))
    await waitFor(() =>
      expect(mocked.setMcpSafetySettings).toHaveBeenCalledWith({
        ...settingsOf(false),
        approvalNotifySound: false,
      }),
    )
  })

  it("saves the timeout on blur, only when it changed", async () => {
    mocked.setMcpSafetySettings.mockResolvedValue({
      status: "ok",
      data: { ...settingsOf(false), approvalTimeoutSeconds: 60 },
    })
    renderSection(false)
    const input = await screen.findByLabelText("Give up after")
    fireEvent.blur(input)
    expect(mocked.setMcpSafetySettings).not.toHaveBeenCalled()

    fireEvent.change(input, { target: { value: "60" } })
    fireEvent.blur(input)
    await waitFor(() =>
      expect(mocked.setMcpSafetySettings).toHaveBeenCalledWith({
        ...settingsOf(false),
        approvalTimeoutSeconds: 60,
      }),
    )
  })
})
