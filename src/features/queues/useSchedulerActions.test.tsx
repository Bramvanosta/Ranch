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
    pauseScheduler: vi.fn(),
    resumeScheduler: vi.fn(),
  },
}))

import { commands } from "@/lib/tauri/bindings"

import { usePauseScheduler, useResumeScheduler } from "./useSchedulerActions"

const mockedPause = commands.pauseScheduler as unknown as ReturnType<typeof vi.fn>
const mockedResume = commands.resumeScheduler as unknown as ReturnType<typeof vi.fn>

const input = {
  connectionId: "conn-x",
  queueRef: { prefix: "bull", name: "emails" },
  schedulerKey: "daily-report",
}

afterEach(() => {
  vi.clearAllMocks()
})

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

describe("usePauseScheduler", () => {
  it("fires pauseScheduler and toasts on success", async () => {
    mockedPause.mockResolvedValue({
      status: "ok",
      data: { paused: true, alreadyInState: false },
    })
    const { result } = renderHook(() => usePauseScheduler(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mockedPause).toHaveBeenCalledWith(input)
    expect(toastSuccess).toHaveBeenCalledWith("Scheduler paused", { duration: 2_000 })
  })

  it("surfaces schedulerNotFound error verbatim", async () => {
    mockedPause.mockResolvedValue({
      status: "error",
      error: {
        kind: "schedulerNotFound",
        message: "Scheduler not found: emails/daily-report",
        docUrl: null,
      },
    })
    const { result } = renderHook(() => usePauseScheduler(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(toastError).toHaveBeenCalledWith(
      expect.stringContaining("Scheduler not found"),
      undefined,
    )
  })

  it("surfaces connectionLocked verbatim with unlock keystroke", async () => {
    mockedPause.mockResolvedValue({
      status: "error",
      error: {
        kind: "connectionLocked",
        message: "Connection prod is locked: Press cmd-shift-U to unlock for this session.",
        docUrl: null,
      },
    })
    const { result } = renderHook(() => usePauseScheduler(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(toastError).toHaveBeenCalledWith(expect.stringContaining("cmd-shift-U"), undefined)
  })
})

describe("useResumeScheduler", () => {
  it("fires resumeScheduler and toasts on success", async () => {
    mockedResume.mockResolvedValue({
      status: "ok",
      data: { paused: false, alreadyInState: false },
    })
    const { result } = renderHook(() => useResumeScheduler(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mockedResume).toHaveBeenCalledWith(input)
    expect(toastSuccess).toHaveBeenCalledWith("Scheduler resumed", { duration: 2_000 })
  })

  it("toasts 'Already running' when idempotent no-op", async () => {
    mockedResume.mockResolvedValue({
      status: "ok",
      data: { paused: false, alreadyInState: true },
    })
    const { result } = renderHook(() => useResumeScheduler(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(toastSuccess).toHaveBeenCalledWith("Already running", { duration: 2_000 })
  })

  it("toasts a generic error for non-locked failures", async () => {
    mockedResume.mockResolvedValue({
      status: "error",
      error: { kind: "redis", message: "boom", docUrl: null },
    })
    const { result } = renderHook(() => useResumeScheduler(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(toastError).toHaveBeenCalledWith(expect.stringContaining("Resume failed"), undefined)
  })
})
