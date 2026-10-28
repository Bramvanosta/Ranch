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
    pauseQueue: vi.fn(),
    resumeQueue: vi.fn(),
  },
}))

import { commands } from "@/lib/tauri/bindings"

import { usePauseQueue, useResumeQueue } from "./mutations"

const mockedPause = commands.pauseQueue as unknown as ReturnType<typeof vi.fn>
const mockedResume = commands.resumeQueue as unknown as ReturnType<typeof vi.fn>

const input = {
  connectionId: "conn-x",
  queueId: "bull:emails",
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

describe("usePauseQueue", () => {
  it("fires pauseQueue and toasts on success", async () => {
    mockedPause.mockResolvedValue({
      status: "ok",
      data: { isPaused: true, alreadyInState: false },
    })
    const { result } = renderHook(() => usePauseQueue(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mockedPause).toHaveBeenCalledWith(input)
    expect(toastSuccess).toHaveBeenCalledWith("Queue paused", { duration: 2_000 })
  })

  it("toasts 'Already paused' when idempotent no-op", async () => {
    mockedPause.mockResolvedValue({
      status: "ok",
      data: { isPaused: true, alreadyInState: true },
    })
    const { result } = renderHook(() => usePauseQueue(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(toastSuccess).toHaveBeenCalledWith("Already paused", { duration: 2_000 })
  })

  it("surfaces the unlock hint verbatim when error is connectionLocked", async () => {
    mockedPause.mockResolvedValue({
      status: "error",
      error: {
        kind: "connectionLocked",
        message: "Connection prod is locked: Press cmd-shift-U to unlock for this session.",
        docUrl: null,
      },
    })
    const { result } = renderHook(() => usePauseQueue(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(toastError).toHaveBeenCalledWith(expect.stringContaining("cmd-shift-U"), undefined)
    expect(toastError.mock.calls[0]?.[0]).not.toContain("Pause failed")
  })

  it("toasts a generic error for non-locked failures", async () => {
    mockedPause.mockResolvedValue({
      status: "error",
      error: { kind: "redis", message: "boom", docUrl: null },
    })
    const { result } = renderHook(() => usePauseQueue(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(toastError).toHaveBeenCalledWith(expect.stringContaining("Pause failed"), undefined)
  })
})

describe("useResumeQueue", () => {
  it("fires resumeQueue and toasts on success", async () => {
    mockedResume.mockResolvedValue({
      status: "ok",
      data: { isPaused: false, alreadyInState: false },
    })
    const { result } = renderHook(() => useResumeQueue(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mockedResume).toHaveBeenCalledWith(input)
    expect(toastSuccess).toHaveBeenCalledWith("Queue resumed", { duration: 2_000 })
  })

  it("toasts 'Already running' when idempotent no-op", async () => {
    mockedResume.mockResolvedValue({
      status: "ok",
      data: { isPaused: false, alreadyInState: true },
    })
    const { result } = renderHook(() => useResumeQueue(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(toastSuccess).toHaveBeenCalledWith("Already running", { duration: 2_000 })
  })
})

describe("optimistic pause", () => {
  const seeded = () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    client.setQueryData(
      ["queues", "conn-x"],
      [
        { id: input.queueId, name: "emails", isPaused: false },
        { id: "bull:images", name: "images", isPaused: false },
      ],
    )
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    )
    const paused = (name: string) =>
      (
        client.getQueryData(["queues", "conn-x"]) as Array<{
          name: string
          isPaused: boolean
        }>
      ).find((q) => q.name === name)?.isPaused
    return { wrapper, paused }
  }

  it("shows the queue paused before Redis answers, and stays busy until refreshed", async () => {
    let answer: (v: unknown) => void = () => {}
    mockedPause.mockReturnValue(new Promise((r) => (answer = r)))
    const { wrapper, paused } = seeded()
    const { result } = renderHook(() => usePauseQueue(), { wrapper })
    act(() => result.current.mutate(input))
    await waitFor(() => expect(paused("emails")).toBe(true))
    expect(paused("images")).toBe(false)
    expect(result.current.isPending).toBe(true)
    answer({ status: "ok", data: { isPaused: true, alreadyInState: false } })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
  })

  it("flips back when the pause fails", async () => {
    mockedPause.mockResolvedValue({
      status: "error",
      error: { kind: "connectionLocked", message: "locked", docUrl: null },
    })
    const { wrapper, paused } = seeded()
    const { result } = renderHook(() => usePauseQueue(), { wrapper })
    await act(async () => {
      result.current.mutate(input)
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(paused("emails")).toBe(false)
  })
})
