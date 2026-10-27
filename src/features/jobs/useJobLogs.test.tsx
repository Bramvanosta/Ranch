import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { getJobLogs: vi.fn() },
}))

import { commands } from "@/lib/tauri/bindings"

import { useJobLogs } from "./useJobLogs"

const mocked = commands as unknown as { getJobLogs: ReturnType<typeof vi.fn> }

afterEach(() => {
  vi.resetAllMocks()
})

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

const queueRef = { prefix: "bull", name: "emails" }

describe("useJobLogs", () => {
  it("returns lines + total on success", async () => {
    mocked.getJobLogs.mockResolvedValue({
      status: "ok",
      data: { lines: ["a", "b", "c"], total: 3, hasMore: false },
    })
    const { result } = renderHook(() => useJobLogs({ connectionId: "c1", queueRef, jobId: "1" }), {
      wrapper: wrapper(),
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data?.lines).toEqual(["a", "b", "c"])
    expect(result.current.data?.total).toBe(3)
  })

  it("treats total=0 as a successful empty result (NOT an error)", async () => {
    mocked.getJobLogs.mockResolvedValue({
      status: "ok",
      data: { lines: [], total: 0, hasMore: false },
    })
    const { result } = renderHook(() => useJobLogs({ connectionId: "c1", queueRef, jobId: "1" }), {
      wrapper: wrapper(),
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data?.total).toBe(0)
    expect(result.current.data?.lines).toEqual([])
  })
})
