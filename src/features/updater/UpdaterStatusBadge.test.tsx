import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { checkForUpdates: vi.fn(), installUpdate: vi.fn() },
}))

// The `updater:progress` listeners, so a test can report a phase as Rust does.
const listeners = vi.hoisted(() => new Map<string, (e: { payload: unknown }) => void>())
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    listeners.set(name, cb)
    return () => listeners.delete(name)
  }),
}))

import { commands, type UpdateProgressEvent } from "@/lib/tauri/bindings"

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

afterEach(() => {
  vi.clearAllMocks()
  listeners.clear()
})

function report(progress: UpdateProgressEvent) {
  const listener = listeners.get("updater:progress")
  if (!listener) throw new Error("nothing listens to updater:progress")
  act(() => listener({ payload: progress }))
}

const MB = 1024 * 1024

/** Open the sheet and click *Install and restart*. */
async function startInstall() {
  render(wrap(<UpdaterStatusBadge />))
  fireEvent.click(await screen.findByTestId("updater-status-available"))
  await waitFor(() => expect(listeners.has("updater:progress")).toBe(true))
  fireEvent.click(await screen.findByTestId("updater-install"))
  await waitFor(() => expect(mocked.installUpdate).toHaveBeenCalledTimes(1))
}

const stateOf = (phase: string) => screen.getByTestId(`updater-step-${phase}`).dataset.state

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
    expect(screen.getByTestId("updater-install")).toHaveTextContent("Downloading")
    expect(screen.getByTestId("updater-install")).toBeDisabled()
    // Nothing to go back to: the app is about to restart.
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled()
  })

  it("follows the install step by step, with the download's bytes and percent", async () => {
    mocked.checkForUpdates.mockResolvedValue(offered())
    mocked.installUpdate.mockReturnValue(new Promise(() => undefined))
    await startInstall()

    // Before the first bytes: the download is under way, its size unknown yet.
    expect(stateOf("downloading")).toBe("running")
    expect(stateOf("verifying")).toBe("pending")

    report({ phase: "downloading", received: 3.5 * MB, total: 14 * MB })
    expect(screen.getByTestId("updater-download-bytes")).toHaveTextContent("3.5 MB of 14 MB")
    const bar = screen.getByRole("progressbar", { name: "Download" })
    expect(bar).toHaveAttribute("aria-valuenow", "25")
    expect(screen.getByText("25%")).toBeInTheDocument()

    report({ phase: "verifying" })
    expect(stateOf("downloading")).toBe("done")
    expect(screen.getByTestId("updater-download-bytes")).toHaveTextContent("14 MB")
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument()
    expect(stateOf("verifying")).toBe("running")
    expect(screen.getByTestId("updater-install")).toHaveTextContent("Checking the signature")

    report({ phase: "installing" })
    expect(stateOf("verifying")).toBe("done")
    expect(stateOf("installing")).toBe("running")
    expect(screen.getByTestId("updater-install")).toHaveTextContent("Installing")

    report({ phase: "restarting" })
    expect(stateOf("installing")).toBe("done")
    expect(stateOf("restarting")).toBe("running")
    expect(screen.getByTestId("updater-install")).toHaveTextContent("Restarting")
    expect(screen.getByTestId("updater-install")).toBeDisabled()
    expect(screen.getByRole("status")).toHaveTextContent("Restarting")
  })

  it("shows a download of unknown size as under way, with the bytes received", async () => {
    mocked.checkForUpdates.mockResolvedValue(offered())
    mocked.installUpdate.mockReturnValue(new Promise(() => undefined))
    await startInstall()

    report({ phase: "downloading", received: 2 * MB, total: null })
    expect(screen.getByTestId("updater-download-bytes")).toHaveTextContent("2.0 MB received")
    expect(screen.getByRole("progressbar", { name: "Download" })).not.toHaveAttribute(
      "aria-valuenow",
    )
    expect(screen.queryByText(/%$/)).not.toBeInTheDocument()
  })

  it("marks the step that failed, says why, and starts over on Try again", async () => {
    mocked.checkForUpdates.mockResolvedValue(offered({ notes: null }))
    let fail: (value: unknown) => void = () => undefined
    mocked.installUpdate.mockReturnValueOnce(new Promise((resolve) => (fail = resolve)))
    await startInstall()

    report({ phase: "downloading", received: 14 * MB, total: 14 * MB })
    report({ phase: "verifying" })
    await act(async () =>
      fail({
        status: "error",
        error: {
          kind: "updater",
          message: "The update's signature could not be verified: signature mismatch",
        },
      }),
    )

    expect(await screen.findByTestId("updater-error")).toHaveTextContent("signature mismatch")
    expect(stateOf("downloading")).toBe("done")
    expect(stateOf("verifying")).toBe("failed")
    expect(stateOf("installing")).toBe("pending")
    const retry = screen.getByTestId("updater-install")
    expect(retry).toHaveTextContent("Try again")
    expect(retry).toBeEnabled()

    // The retry starts from the top: the failed step is no longer shown.
    mocked.installUpdate.mockReturnValueOnce(new Promise(() => undefined))
    fireEvent.click(retry)
    await waitFor(() => expect(mocked.installUpdate).toHaveBeenCalledTimes(2))
    expect(screen.queryByTestId("updater-error")).not.toBeInTheDocument()
    expect(stateOf("downloading")).toBe("running")
    expect(stateOf("verifying")).toBe("pending")
  })

  it("starts one install, however many times the button is clicked", async () => {
    mocked.checkForUpdates.mockResolvedValue(offered())
    mocked.installUpdate.mockReturnValue(new Promise(() => undefined))
    await startInstall()
    fireEvent.click(screen.getByTestId("updater-install"))
    fireEvent.click(screen.getByTestId("updater-install"))
    expect(mocked.installUpdate).toHaveBeenCalledTimes(1)
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
    expect(screen.getByTestId("updater-install")).toHaveTextContent("Try again")
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
