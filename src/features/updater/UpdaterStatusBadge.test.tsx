import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { checkForUpdates: vi.fn(), installUpdate: vi.fn() },
}))

import { commands } from "@/lib/tauri/bindings"

import { UpdaterStatusBadge } from "./UpdaterStatusBadge"

const mocked = commands as unknown as {
  checkForUpdates: ReturnType<typeof vi.fn>
  installUpdate: ReturnType<typeof vi.fn>
}

const offered = (over: Record<string, unknown> = {}) => ({
  status: "ok",
  data: {
    available: true,
    version: "0.3.0",
    notes: "### Fixed\n- Waiting jobs were missing from the lists.",
    currentVersion: "0.2.0",
    disabled: false,
    disabledReason: null,
    ...over,
  },
})

function wrap(child: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return <QueryClientProvider client={client}>{child}</QueryClientProvider>
}

afterEach(() => vi.resetAllMocks())

describe("UpdaterStatusBadge", () => {
  it("renders nothing when no update is available and updates are enabled", async () => {
    mocked.checkForUpdates.mockResolvedValue({
      status: "ok",
      data: {
        available: false,
        version: null,
        currentVersion: "0.1.0",
        disabled: false,
        disabledReason: null,
      },
    })
    const { container } = render(wrap(<UpdaterStatusBadge />))
    await waitFor(() => expect(mocked.checkForUpdates).toHaveBeenCalled())
    expect(container.querySelector('[data-testid^="updater-status-"]')).toBeNull()
  })

  it("renders the disabled badge with the disable reason as a tooltip", async () => {
    mocked.checkForUpdates.mockResolvedValue({
      status: "ok",
      data: {
        available: false,
        version: null,
        currentVersion: "0.1.0",
        disabled: true,
        disabledReason: "Disabled via RANCH_DISABLE_UPDATE_CHECK environment variable",
      },
    })
    render(wrap(<UpdaterStatusBadge />))
    await waitFor(() => {
      expect(screen.getByTestId("updater-status-disabled")).toBeInTheDocument()
    })
    expect(screen.getByTestId("updater-status-disabled")).toHaveAttribute(
      "title",
      expect.stringContaining("RANCH_DISABLE_UPDATE_CHECK"),
    )
  })

  it("renders the available badge with the new version", async () => {
    mocked.checkForUpdates.mockResolvedValue({
      status: "ok",
      data: {
        available: true,
        version: "0.2.0",
        currentVersion: "0.1.0",
        disabled: false,
        disabledReason: null,
      },
    })
    render(wrap(<UpdaterStatusBadge />))
    await waitFor(() => {
      expect(screen.getByTestId("updater-status-available")).toBeInTheDocument()
    })
    expect(screen.getByTestId("updater-status-available")).toHaveTextContent("v0.2.0")
  })

  it("installs only after the user confirms, and shows the release notes first", async () => {
    mocked.checkForUpdates.mockResolvedValue(offered())
    // On success the app restarts: the call never comes back.
    mocked.installUpdate.mockReturnValue(new Promise(() => undefined))
    render(wrap(<UpdaterStatusBadge />))

    fireEvent.click(await screen.findByTestId("updater-status-available"))
    expect(mocked.installUpdate).not.toHaveBeenCalled()
    const dialog = await screen.findByTestId("updater-confirm")
    expect(dialog).toHaveTextContent("Update Ranch to v0.3.0?")
    expect(dialog).toHaveTextContent("You are on v0.2.0")
    expect(screen.getByTestId("updater-notes")).toHaveTextContent("Waiting jobs were missing")

    fireEvent.click(screen.getByTestId("updater-install"))
    await waitFor(() => expect(mocked.installUpdate).toHaveBeenCalledTimes(1))
    expect(screen.getByTestId("updater-install")).toHaveTextContent("Installing")
    expect(screen.getByTestId("updater-install")).toBeDisabled()
  })

  it("says what went wrong when the install fails, and lets the user try again", async () => {
    mocked.checkForUpdates.mockResolvedValue(offered({ notes: null }))
    mocked.installUpdate.mockResolvedValue({
      status: "error",
      error: { kind: "updater", message: "The update could not be installed: signature mismatch" },
    })
    render(wrap(<UpdaterStatusBadge />))
    fireEvent.click(await screen.findByTestId("updater-status-available"))
    expect(screen.queryByTestId("updater-notes")).not.toBeInTheDocument()
    fireEvent.click(await screen.findByTestId("updater-install"))
    expect(await screen.findByTestId("updater-error")).toHaveTextContent("signature mismatch")
    expect(screen.getByTestId("updater-install")).toBeEnabled()
  })

  it("stays silent when the feed cannot be read (offline, host down)", async () => {
    mocked.checkForUpdates.mockResolvedValue({
      status: "error",
      error: { kind: "updater", message: "Cannot read the update feed: timeout" },
    })
    const { container } = render(wrap(<UpdaterStatusBadge />))
    await waitFor(() => expect(mocked.checkForUpdates).toHaveBeenCalledTimes(1))
    expect(container.querySelector('[data-testid^="updater-"]')).toBeNull()
  })
})
