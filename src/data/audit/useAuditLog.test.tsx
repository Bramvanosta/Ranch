import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listAuditLogEntries: vi.fn() },
}))

import { commands } from "@/lib/tauri/bindings"

import { AUDIT_POLL_MS, useAuditLog } from "./queries"

const mocked = commands.listAuditLogEntries as unknown as ReturnType<typeof vi.fn>

afterEach(() => {
  vi.resetAllMocks()
})

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

describe("useAuditLog", () => {
  it("forwards filter args (nulled when empty) to commands.listAuditLogEntries", async () => {
    mocked.mockResolvedValue({ status: "ok", data: [] })
    const { result } = renderHook(
      () =>
        useAuditLog({
          from: "2026-05-01T00:00:00Z",
          to: "2026-05-11T23:59:59Z",
          connectionId: "conn-x",
          action: "retryJob",
          source: "ui",
        }),
      { wrapper: wrap() },
    )
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mocked).toHaveBeenCalledWith({
      from: "2026-05-01T00:00:00Z",
      to: "2026-05-11T23:59:59Z",
      connectionId: "conn-x",
      action: "retryJob",
      source: "ui",
      offset: 0,
      limit: 200,
    })
  })

  it("passes null for unset filters", async () => {
    mocked.mockResolvedValue({ status: "ok", data: [] })
    const { result } = renderHook(() => useAuditLog(), { wrapper: wrap() })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mocked).toHaveBeenCalledWith(
      expect.objectContaining({
        from: null,
        to: null,
        connectionId: null,
        action: null,
        source: null,
      }),
    )
  })

  // An agent's writes are inserted by another
  // process, so nothing in this one invalidates the query: the page polls.
  it("polls while it is mounted", async () => {
    vi.useFakeTimers()
    try {
      mocked.mockResolvedValue({ status: "ok", data: [] })
      const { result } = renderHook(() => useAuditLog(), { wrapper: wrap() })
      await vi.waitFor(() => expect(result.current.isSuccess).toBe(true))
      expect(mocked).toHaveBeenCalledTimes(1)
      await vi.advanceTimersByTimeAsync(AUDIT_POLL_MS + 100)
      expect(mocked).toHaveBeenCalledTimes(2)
      await vi.advanceTimersByTimeAsync(AUDIT_POLL_MS + 100)
      expect(mocked).toHaveBeenCalledTimes(3)
    } finally {
      vi.useRealTimers()
    }
  })

  it("surfaces the error message when the command returns an error", async () => {
    mocked.mockResolvedValue({
      status: "error",
      error: { kind: "database", message: "disk full", docUrl: null },
    })
    const { result } = renderHook(() => useAuditLog(), { wrapper: wrap() })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect((result.current.error as Error).message).toBe("disk full")
  })
})
