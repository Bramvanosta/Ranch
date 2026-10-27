import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

const toastSuccess = vi.fn()
const toastError = vi.fn()
vi.mock("sonner", () => ({
  toast: {
    success: (msg: string, opts?: unknown) => toastSuccess(msg, opts),
    error: (msg: string, opts?: unknown) => toastError(msg, opts),
  },
}))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: { exportAuditLog: vi.fn() },
}))

import { commands } from "@/lib/tauri/bindings"

import { useAuditExport } from "./useAuditExport"

const mocked = commands.exportAuditLog as unknown as ReturnType<typeof vi.fn>

afterEach(() => {
  vi.clearAllMocks()
})

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

describe("useAuditExport", () => {
  it("calls the export command with the right format and triggers a download on success", async () => {
    mocked.mockResolvedValue({
      status: "ok",
      data: {
        content: "[]",
        suggestedFilename: "ranch-audit-2026-05-11-to-2026-05-11.json",
      },
    })
    // Stub URL.createObjectURL + revokeObjectURL so the test runs in jsdom.
    const createObjectURL = vi.fn().mockReturnValue("blob:test")
    const revokeObjectURL = vi.fn()
    Object.assign(URL, { createObjectURL, revokeObjectURL })

    const { result } = renderHook(() => useAuditExport(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate({ format: "json" })
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    expect(mocked).toHaveBeenCalledWith(expect.objectContaining({ format: "json" }))
    expect(createObjectURL).toHaveBeenCalled()
    expect(revokeObjectURL).toHaveBeenCalled()
    expect(toastSuccess).toHaveBeenCalledWith(
      expect.stringContaining("ranch-audit-2026-05-11-to-2026-05-11.json"),
      expect.anything(),
    )
  })

  it("toasts an error when the export command fails", async () => {
    mocked.mockResolvedValue({
      status: "error",
      error: { kind: "invalidInput", message: "Unsupported format", docUrl: null },
    })
    const { result } = renderHook(() => useAuditExport(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate({ format: "json" })
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(toastError).toHaveBeenCalledWith(
      expect.stringContaining("Unsupported format"),
      expect.anything(),
    )
  })
})
