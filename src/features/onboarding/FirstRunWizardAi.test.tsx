/**
 * The first-run wizard asks whether to use AI agents, with Yes
 * and No as equal choices.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    setKvSetting: vi.fn(),
    getMcpInstallerStatus: vi.fn(),
    setAiFeatures: vi.fn(),
  },
}))
// Step 1 is the connection sheet's own business: saved at once here.
vi.mock("@/features/connections/ConnectionSheet", () => ({
  ConnectionSheet: ({ open, onSaved }: { open: boolean; onSaved: (id: string) => void }) =>
    open ? (
      <button type="button" onClick={() => onSaved("c1")}>
        save connection
      </button>
    ) : null,
}))
vi.mock("@/features/mcp/McpSetupCards", () => ({ McpSetupCards: () => <p>installers</p> }))

import { useAiStore } from "@/lib/ai/aiFeatures"
import { commands } from "@/lib/tauri/bindings"

import { FirstRunWizard } from "./FirstRunWizard"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>
const ok = <T,>(data: T) => ({ status: "ok", data })

afterEach(() => {
  vi.resetAllMocks()
  useAiStore.getState().setEnabled(true)
})

function wrap(node: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>)
}

function reachStepTwo(onClose = vi.fn()) {
  wrap(<FirstRunWizard onClose={onClose} />)
  fireEvent.click(screen.getByTestId("wizard-add-connection"))
  fireEvent.click(screen.getByRole("button", { name: "save connection" }))
  return onClose
}

describe("FirstRunWizard: AI agents", () => {
  it("asks with Yes and No as equal choices", () => {
    reachStepTwo()
    expect(screen.getByText("Use Ranch with AI agents?")).toBeInTheDocument()
    const yes = screen.getByTestId("wizard-ai-yes")
    const no = screen.getByTestId("wizard-ai-no")
    expect(yes.className).toBe(no.className)
  })

  it("Yes turns AI features on and shows the agent installers", async () => {
    mocked.setAiFeatures.mockResolvedValue(ok(true))
    mocked.getMcpInstallerStatus.mockResolvedValue(ok({}))
    reachStepTwo()
    fireEvent.click(screen.getByTestId("wizard-ai-yes"))
    await waitFor(() => expect(mocked.setAiFeatures).toHaveBeenCalledWith(true))
    expect(await screen.findByText("installers")).toBeInTheDocument()
  })

  it("No turns AI features off and closes the wizard", async () => {
    mocked.setAiFeatures.mockResolvedValue(ok(false))
    mocked.setKvSetting.mockResolvedValue(ok(null))
    const onClose = reachStepTwo()
    fireEvent.click(screen.getByTestId("wizard-ai-no"))
    await waitFor(() => expect(onClose).toHaveBeenCalled())
    expect(mocked.setAiFeatures).toHaveBeenCalledWith(false)
    expect(useAiStore.getState().enabled).toBe(false)
    expect(mocked.getMcpInstallerStatus).not.toHaveBeenCalled()
  })
})
