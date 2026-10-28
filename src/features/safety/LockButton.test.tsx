import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { bullmqConnection, redisEndpoint } from "@/domain/bullmqEndpoint"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    getConnectionLockState: vi.fn(),
    unlockConnection: vi.fn(),
    lockConnection: vi.fn(),
    listConnections: vi.fn(),
  },
}))

import { type Connection, commands } from "@/lib/tauri/bindings"
import { pressChord } from "@/test/pressChord"

import { LockButton } from "./LockButton"

const mockedGet = commands.getConnectionLockState as unknown as ReturnType<typeof vi.fn>
const mockedUnlock = commands.unlockConnection as unknown as ReturnType<typeof vi.fn>
const mockedLock = commands.lockConnection as unknown as ReturnType<typeof vi.fn>
const mockedListConns = commands.listConnections as unknown as ReturnType<typeof vi.fn>

function conn(id: string, envColor: string): Connection {
  return {
    id,
    environmentId: "env-1",
    groupId: "g-1",
    name: `Connection ${id}`,
    ...bullmqConnection(redisEndpoint({ host: "127.0.0.1" })),
    envColor,
    envName: envColor === "env-production" ? "Production" : "Local",
    envLabel: envColor === "env-production" ? "Production" : "Local",
    groupName: "Default",
    createdAt: "2026-05-11T12:00:00Z",
    mcpReadonly: null,
    sidebarCollapsed: false,
  }
}

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

beforeEach(() => {
  // Default platform = mac for keystroke hint tests.
  Object.defineProperty(navigator, "platform", { value: "MacIntel", configurable: true })
})

afterEach(() => {
  vi.resetAllMocks()
})

describe("LockButton", () => {
  it("locked: Read-only with the ⇧⌘U hint", async () => {
    mockedListConns.mockResolvedValue({ status: "ok", data: [conn("conn-x", "env-production")] })
    mockedGet.mockResolvedValue({ status: "ok", data: "locked" })

    const Wrap = wrap()
    render(
      <Wrap>
        <LockButton connectionId="conn-x" />
      </Wrap>,
    )
    const banner = await screen.findByTestId("lock-button")
    expect(banner).toHaveAttribute("data-state", "locked")
    expect(banner).toHaveTextContent("Read-only")
    expect(banner).toHaveTextContent("⇧⌘U")
  })

  it("unlocked: how long it has been open", async () => {
    mockedListConns.mockResolvedValue({ status: "ok", data: [conn("conn-x", "env-production")] })
    mockedGet.mockResolvedValue({ status: "ok", data: "unlocked" })

    const Wrap = wrap()
    render(
      <Wrap>
        <LockButton connectionId="conn-x" />
      </Wrap>,
    )
    const banner = await screen.findByTestId("lock-button")
    await waitFor(() => expect(banner).toHaveAttribute("data-state", "unlocked"))
    // How long it has been open, by the minute.
    expect(banner).toHaveTextContent("Unlocked · 0 min")
  })

  it("uses the Ctrl+Shift+U hint on non-mac platforms", async () => {
    Object.defineProperty(navigator, "platform", { value: "Linux x86_64", configurable: true })
    mockedListConns.mockResolvedValue({ status: "ok", data: [conn("conn-x", "env-production")] })
    mockedGet.mockResolvedValue({ status: "ok", data: "locked" })

    const Wrap = wrap()
    render(
      <Wrap>
        <LockButton connectionId="conn-x" />
      </Wrap>,
    )
    const banner = await screen.findByTestId("lock-button")
    expect(banner).toHaveTextContent("Ctrl+Shift+U")
  })

  it("shows nothing for a connection that is not production", async () => {
    mockedListConns.mockResolvedValue({ status: "ok", data: [conn("conn-x", "env-local")] })
    mockedGet.mockResolvedValue({ status: "ok", data: "unlocked" })

    const Wrap = wrap()
    const { container } = render(
      <Wrap>
        <LockButton connectionId="conn-x" />
      </Wrap>,
    )
    // Wait a tick for the cached query to resolve, then assert nothing
    // rendered.
    await waitFor(() => {
      expect(mockedListConns).toHaveBeenCalled()
    })
    expect(container.querySelector("[data-testid='lock-button']")).toBeNull()
  })

  it("clicking the unlock button fires unlockConnection", async () => {
    mockedListConns.mockResolvedValue({ status: "ok", data: [conn("conn-x", "env-production")] })
    mockedGet.mockResolvedValue({ status: "ok", data: "locked" })
    mockedUnlock.mockResolvedValue({ status: "ok", data: "unlocked" })

    const Wrap = wrap()
    render(
      <Wrap>
        <LockButton connectionId="conn-x" />
      </Wrap>,
    )
    const button = await screen.findByTestId("lock-button")
    await act(async () => {
      fireEvent.click(button)
    })
    await waitFor(() => expect(mockedUnlock).toHaveBeenCalledWith({ connectionId: "conn-x" }))
  })

  it("clicking the relock button fires lockConnection", async () => {
    mockedListConns.mockResolvedValue({ status: "ok", data: [conn("conn-x", "env-production")] })
    mockedGet.mockResolvedValue({ status: "ok", data: "unlocked" })
    mockedLock.mockResolvedValue({ status: "ok", data: "locked" })

    const Wrap = wrap()
    render(
      <Wrap>
        <LockButton connectionId="conn-x" />
      </Wrap>,
    )
    await waitFor(() =>
      expect(screen.getByTestId("lock-button")).toHaveAttribute("data-state", "unlocked"),
    )
    await act(async () => {
      fireEvent.click(screen.getByTestId("lock-button"))
    })
    await waitFor(() => expect(mockedLock).toHaveBeenCalledWith({ connectionId: "conn-x" }))
  })

  it("⇧⌘U (Connection › Unlock for Session) fires unlockConnection on locked state", async () => {
    mockedListConns.mockResolvedValue({ status: "ok", data: [conn("conn-x", "env-production")] })
    mockedGet.mockResolvedValue({ status: "ok", data: "locked" })
    mockedUnlock.mockResolvedValue({ status: "ok", data: "unlocked" })

    const Wrap = wrap()
    render(
      <Wrap>
        <LockButton connectionId="conn-x" />
      </Wrap>,
    )
    // Wait for the locked banner to mount (so the shortcut is registered).
    await screen.findByTestId("lock-button")

    await act(async () => {
      pressChord("cmd+shift+u")
    })
    await waitFor(() => expect(mockedUnlock).toHaveBeenCalledWith({ connectionId: "conn-x" }))
  })
})
