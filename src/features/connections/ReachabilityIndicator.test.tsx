import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { pingConnection: vi.fn() },
}))

import { TooltipProvider } from "@/components/ui/tooltip"
import { commands } from "@/lib/tauri/bindings"
import { ReachabilityIndicator } from "./ReachabilityIndicator"

const mocked = commands as unknown as { pingConnection: ReturnType<typeof vi.fn> }

afterEach(() => {
  vi.resetAllMocks()
})

function renderIndicator() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <ReachabilityIndicator connectionId="c1" />
      </TooltipProvider>
    </QueryClientProvider>,
  )
}

describe("ReachabilityIndicator", () => {
  it("renders a 'Pinging…' status while loading", () => {
    mocked.pingConnection.mockReturnValue(new Promise(() => {})) // never resolves
    renderIndicator()
    expect(screen.getByLabelText(/Pinging/i)).toBeInTheDocument()
  })

  it("renders latency in ms when reachable", async () => {
    mocked.pingConnection.mockResolvedValue({
      status: "ok",
      data: { success: true, latencyMs: 12, error: null },
    })

    renderIndicator()

    await waitFor(() => {
      expect(screen.getByLabelText(/Reachable, 12 ms/i)).toBeInTheDocument()
    })
    expect(screen.getByText("12 ms")).toBeInTheDocument()
  })

  it("renders an unreachable indicator with classified error label when ping fails", async () => {
    mocked.pingConnection.mockResolvedValue({
      status: "ok",
      data: {
        success: false,
        latencyMs: null,
        error: { kind: "authFailed" },
      },
    })

    renderIndicator()

    await waitFor(() => {
      expect(screen.getByLabelText(/Unreachable: Auth failed/i)).toBeInTheDocument()
    })
  })

  it("renders an unreachable indicator when the outer command call errors", async () => {
    mocked.pingConnection.mockResolvedValue({
      status: "error",
      error: { kind: "connectionNotFound", message: "gone", docUrl: null },
    })

    renderIndicator()

    await waitFor(() => {
      expect(screen.getByLabelText(/Unreachable: gone/i)).toBeInTheDocument()
    })
  })
})
