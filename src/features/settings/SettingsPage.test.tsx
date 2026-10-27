import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    getKvSetting: vi.fn(),
    setKvSetting: vi.fn(),
    checkForUpdates: vi.fn(),
    installUpdate: vi.fn(),
    setAiFeatures: vi.fn(),
  },
}))
vi.mock("next-themes", () => ({ useTheme: () => ({ theme: "system", setTheme: vi.fn() }) }))
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
const runtime = vi.hoisted(() => ({ agents: true, updater: true }))
vi.mock("@/lib/runtime/runtime", () => ({
  hasAgents: () => runtime.agents,
  hasUpdater: () => runtime.updater,
  hasOverlayTitleBar: () => false,
  getRuntime: () => "preview",
}))
// The Agents and Alerts pages have their own tests.
vi.mock("./AgentsSetup", () => ({ AgentsSetup: () => <p>agents setup</p> }))
vi.mock("@/features/notifications/NotificationThresholdsTable", () => ({
  NotificationThresholdsTable: () => <p>thresholds</p>,
}))

import { useAiStore } from "@/lib/ai/aiFeatures"
import { commands } from "@/lib/tauri/bindings"
import { SettingsPage } from "./SettingsPage"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>
const ok = <T,>(data: T) => ({ status: "ok", data })

afterEach(() => {
  vi.resetAllMocks()
  useAiStore.getState().setEnabled(true)
  runtime.agents = true
  runtime.updater = true
})

function wrap(node: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>)
}

describe("SettingsPage", () => {
  it("has the toolbar tabs of a Mac app's settings, AI features where there can be agents", () => {
    mocked.getKvSetting.mockResolvedValue(ok(null))
    wrap(<SettingsPage tab="general" onTab={vi.fn()} />)
    const tabs = screen.getAllByRole("tab").map((t) => t.textContent)
    expect(tabs).toEqual([
      "General",
      "Appearance",
      "Refresh",
      "Alerts",
      "Updates",
      "Advanced",
      "AI features",
    ])
    expect(screen.getByRole("tab", { name: "General" })).toHaveAttribute("aria-selected", "true")
  })

  it("has no AI features tab in the server image", () => {
    runtime.agents = false
    mocked.getKvSetting.mockResolvedValue(ok(null))
    wrap(<SettingsPage tab="general" onTab={vi.fn()} />)
    expect(screen.queryByRole("tab", { name: "AI features" })).not.toBeInTheDocument()
  })

  it("AI features turns agents off, and the pointer to the Agents page goes with it (16.1)", async () => {
    mocked.setAiFeatures.mockResolvedValue(ok(false))
    wrap(<SettingsPage tab="ai" onTab={vi.fn()} />)
    expect(screen.getByTestId("settings-ai-agents-page")).toBeInTheDocument()
    fireEvent.click(screen.getByRole("switch", { name: "Use Ranch with AI agents" }))
    await waitFor(() => expect(mocked.setAiFeatures).toHaveBeenCalledWith(false))
    await waitFor(() =>
      expect(screen.queryByTestId("settings-ai-agents-page")).not.toBeInTheDocument(),
    )
    expect(screen.getByRole("switch", { name: "Use Ranch with AI agents" })).not.toBeChecked()
    // The tab stays, to turn them back on.
    expect(screen.getByRole("tab", { name: "AI features" })).toBeInTheDocument()
  })

  it("chooses what job data agents see, masking by default", async () => {
    mocked.getKvSetting.mockResolvedValue(ok(null))
    mocked.setKvSetting.mockResolvedValue(ok(null))
    wrap(<SettingsPage tab="ai" onTab={vi.fn()} />)
    const mask = await screen.findByRole("radio", { name: /Mask fields whose name matches/ })
    expect(mask).toBeChecked()
    const patterns = screen.getByLabelText(/Field names to mask/) as HTMLTextAreaElement
    await waitFor(() =>
      expect(patterns.value).toBe("email, phone, token, password, secret, iban, card*"),
    )
    fireEvent.change(patterns, { target: { value: "email, ssn" } })
    fireEvent.blur(patterns)
    await waitFor(() =>
      expect(mocked.setKvSetting).toHaveBeenCalledWith("ai_share_patterns", "email, ssn"),
    )
    fireEvent.click(screen.getByRole("radio", { name: /Names, states and errors only/ }))
    await waitFor(() => expect(mocked.setKvSetting).toHaveBeenCalledWith("ai_share_mode", "names"))
  })

  it("a tab asks for its page", () => {
    mocked.getKvSetting.mockResolvedValue(ok(null))
    const onTab = vi.fn()
    wrap(<SettingsPage tab="general" onTab={onTab} />)
    fireEvent.click(screen.getByRole("tab", { name: "Alerts" }))
    expect(onTab).toHaveBeenCalledWith("alerts")
  })

  it("General sets the jobs per page and unmutes everything", async () => {
    mocked.getKvSetting.mockImplementation(async (key: string) =>
      ok(key === "attention_mutes" ? JSON.stringify({ a: Date.now() + 60_000 }) : null),
    )
    mocked.setKvSetting.mockResolvedValue(ok(null))
    wrap(<SettingsPage tab="general" onTab={vi.fn()} />)
    expect(await screen.findByText("1 item is hidden until its mute ends.")).toBeInTheDocument()
    fireEvent.click(screen.getByRole("radio", { name: "100" }))
    await waitFor(() => expect(mocked.setKvSetting).toHaveBeenCalledWith("jobs.pageSize", "100"))
    fireEvent.click(screen.getByRole("button", { name: "Unmute all" }))
    await waitFor(() => expect(mocked.setKvSetting).toHaveBeenCalledWith("attention_mutes", "{}"))
  })

  it("Updates says the version is current, and checks again on demand", async () => {
    mocked.checkForUpdates.mockResolvedValue(
      ok({ available: false, version: null, currentVersion: "0.9.0", disabled: false }),
    )
    wrap(<SettingsPage tab="updates" onTab={vi.fn()} />)
    expect(await screen.findByText("Ranch 0.9.0")).toBeInTheDocument()
    await waitFor(() =>
      expect(screen.getByTestId("settings-update-status")).toHaveTextContent(
        "Ranch is up to date.",
      ),
    )
    fireEvent.click(screen.getByRole("button", { name: "Check now" }))
    await waitFor(() => expect(mocked.checkForUpdates).toHaveBeenCalledTimes(2))
  })

  it("Updates offers to install a published version after a confirmation", async () => {
    mocked.checkForUpdates.mockResolvedValue(
      ok({
        available: true,
        version: "1.0.0",
        notes: "Faster lists.",
        currentVersion: "0.9.0",
        disabled: false,
      }),
    )
    mocked.installUpdate.mockReturnValue(new Promise(() => {}))
    wrap(<SettingsPage tab="updates" onTab={vi.fn()} />)
    fireEvent.click(await screen.findByRole("button", { name: "Install and restart" }))
    expect(screen.getByText("Faster lists.")).toBeInTheDocument()
    expect(mocked.installUpdate).not.toHaveBeenCalled()
    const both = screen.getAllByRole("button", { name: "Install and restart" })
    fireEvent.click(both[both.length - 1] as HTMLElement)
    await waitFor(() => expect(mocked.installUpdate).toHaveBeenCalled())
  })

  it("Updates in the server image says to pull a newer tag", () => {
    runtime.updater = false
    wrap(<SettingsPage tab="updates" onTab={vi.fn()} />)
    expect(screen.getByText(/pull a newer tag/)).toBeInTheDocument()
    expect(mocked.checkForUpdates).not.toHaveBeenCalled()
  })

  it("Advanced puts back the hints and the welcome screen", async () => {
    mocked.setKvSetting.mockResolvedValue(ok(null))
    wrap(<SettingsPage tab="advanced" onTab={vi.fn()} />)
    const [hints, , welcome] = screen
      .getAllByRole("button")
      .filter((b) => /Show again|Clear/.test(b.textContent ?? ""))
    fireEvent.click(hints as HTMLElement)
    await waitFor(() =>
      expect(mocked.setKvSetting).toHaveBeenCalledWith("hint:cross-queue-scope:dismissed", "0"),
    )
    fireEvent.click(welcome as HTMLElement)
    await waitFor(() => expect(mocked.setKvSetting).toHaveBeenCalledWith("wizard.dismissed", "0"))
  })
})
