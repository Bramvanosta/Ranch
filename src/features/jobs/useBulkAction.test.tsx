import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    bulkRetryJobs: vi.fn(),
    bulkRemoveJobs: vi.fn(),
    bulkPromoteJobs: vi.fn(),
  },
}))

import { commands } from "@/lib/tauri/bindings"

import { useBulkPromote, useBulkRemove, useBulkRetry } from "./useBulkAction"

const mockedRetry = commands.bulkRetryJobs as unknown as ReturnType<typeof vi.fn>
const mockedRemove = commands.bulkRemoveJobs as unknown as ReturnType<typeof vi.fn>
const mockedPromote = commands.bulkPromoteJobs as unknown as ReturnType<typeof vi.fn>

afterEach(() => {
  vi.clearAllMocks()
})

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

const input = {
  connectionId: "conn-x",
  queueRef: { prefix: "bull", name: "emails" },
  jobIds: ["1", "2", "3"],
}

describe("useBulkRetry", () => {
  it("resolves with the BulkActionResult on success", async () => {
    mockedRetry.mockResolvedValue({
      status: "ok",
      data: { action: "retry", total: 3, succeeded: 3, failed: [], cancelled: false },
    })
    const { result } = renderHook(() => useBulkRetry(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data?.succeeded).toBe(3)
    expect(mockedRetry).toHaveBeenCalledWith(input)
  })

  it("throws with kind=connectionLocked on locked-connection error", async () => {
    mockedRetry.mockResolvedValue({
      status: "error",
      error: {
        kind: "connectionLocked",
        message: "Connection prod is locked: Press cmd-shift-U to unlock for this session.",
        docUrl: null,
      },
    })
    const { result } = renderHook(() => useBulkRetry(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    const err = result.current.error as Error & { kind?: string }
    expect(err.kind).toBe("connectionLocked")
    expect(err.message).toContain("cmd-shift-U")
  })
})

describe("useBulkRemove", () => {
  it("resolves with the BulkActionResult on success", async () => {
    mockedRemove.mockResolvedValue({
      status: "ok",
      data: { action: "remove", total: 3, succeeded: 3, failed: [], cancelled: false },
    })
    const { result } = renderHook(() => useBulkRemove(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mockedRemove).toHaveBeenCalledWith(input)
  })
})

describe("useBulkPromote", () => {
  it("resolves with the BulkActionResult on success", async () => {
    mockedPromote.mockResolvedValue({
      status: "ok",
      data: { action: "promote", total: 2, succeeded: 2, failed: [], cancelled: false },
    })
    const { result } = renderHook(() => useBulkPromote(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mockedPromote).toHaveBeenCalledWith(input)
  })

  it("preserves the error kind on partial failure errors", async () => {
    mockedPromote.mockResolvedValue({
      status: "error",
      error: { kind: "redis", message: "boom", docUrl: null },
    })
    const { result } = renderHook(() => useBulkPromote(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    const err = result.current.error as Error & { kind?: string }
    expect(err.kind).toBe("redis")
  })
})
