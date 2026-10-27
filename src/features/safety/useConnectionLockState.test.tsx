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
  commands: {
    getConnectionLockState: vi.fn(),
    unlockConnection: vi.fn(),
    lockConnection: vi.fn(),
  },
}))

import { commands } from "@/lib/tauri/bindings"

import {
  useConnectionLockState,
  useLockConnection,
  useUnlockConnection,
} from "./useConnectionLockState"

const mockedGet = commands.getConnectionLockState as unknown as ReturnType<typeof vi.fn>
const mockedUnlock = commands.unlockConnection as unknown as ReturnType<typeof vi.fn>
const mockedLock = commands.lockConnection as unknown as ReturnType<typeof vi.fn>

afterEach(() => {
  vi.clearAllMocks()
})

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

describe("useConnectionLockState", () => {
  it("fetches the actor's lock state via getConnectionLockState", async () => {
    mockedGet.mockResolvedValue({ status: "ok", data: "locked" })
    const { result } = renderHook(() => useConnectionLockState("conn-x"), { wrapper: wrap() })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mockedGet).toHaveBeenCalledWith({ connectionId: "conn-x" })
    expect(result.current.data).toBe("locked")
  })

  it("surfaces backend errors via the query's error state", async () => {
    mockedGet.mockResolvedValue({
      status: "error",
      error: { kind: "connectionNotFound", message: "Connection not found: conn-x", docUrl: null },
    })
    const { result } = renderHook(() => useConnectionLockState("conn-x"), { wrapper: wrap() })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect((result.current.error as Error).message).toContain("Connection not found")
  })
})

describe("useUnlockConnection", () => {
  it("fires unlockConnection and toasts on success", async () => {
    mockedUnlock.mockResolvedValue({ status: "ok", data: "unlocked" })
    const { result } = renderHook(() => useUnlockConnection(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate("conn-x")
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mockedUnlock).toHaveBeenCalledWith({ connectionId: "conn-x" })
    expect(toastSuccess).toHaveBeenCalledWith(
      expect.stringContaining("Unlocked"),
      expect.anything(),
    )
  })

  it("toasts an error when unlockConnection fails", async () => {
    mockedUnlock.mockResolvedValue({
      status: "error",
      error: { kind: "redis", message: "Redis unreachable", docUrl: null },
    })
    const { result } = renderHook(() => useUnlockConnection(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate("conn-x")
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(toastError).toHaveBeenCalledWith(expect.stringContaining("Redis unreachable"), undefined)
  })
})

describe("useLockConnection", () => {
  it("fires lockConnection and toasts on success", async () => {
    mockedLock.mockResolvedValue({ status: "ok", data: "locked" })
    const { result } = renderHook(() => useLockConnection(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate("conn-x")
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mockedLock).toHaveBeenCalledWith({ connectionId: "conn-x" })
    expect(toastSuccess).toHaveBeenCalledWith(
      expect.stringContaining("Re-locked"),
      expect.anything(),
    )
  })

  it("toasts an error when lockConnection fails", async () => {
    mockedLock.mockResolvedValue({
      status: "error",
      error: { kind: "redis", message: "boom", docUrl: null },
    })
    const { result } = renderHook(() => useLockConnection(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate("conn-x")
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(toastError).toHaveBeenCalledWith(expect.stringContaining("boom"), undefined)
  })
})
