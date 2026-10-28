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
    retryJob: vi.fn(),
    removeJob: vi.fn(),
    promoteJob: vi.fn(),
  },
}))

import { commands, type Job } from "@/lib/tauri/bindings"

import { usePromoteJob, useRemoveJob, useRetryJob } from "./mutations"

const mockedRetry = commands.retryJob as unknown as ReturnType<typeof vi.fn>
const mockedRemove = commands.removeJob as unknown as ReturnType<typeof vi.fn>
const mockedPromote = commands.promoteJob as unknown as ReturnType<typeof vi.fn>

afterEach(() => {
  vi.clearAllMocks()
})

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

const baseInput = {
  connectionId: "conn-x",
  queueId: "bull:emails",
  jobId: "42",
}

// ─── useRetryJob ─────────────────────────────────────────────────────────

describe("useRetryJob", () => {
  it("fires retryJob and toasts on success", async () => {
    mockedRetry.mockResolvedValue({
      status: "ok",
      data: { jobId: "42", newState: "waiting", action: "retry" },
    })
    const { result } = renderHook(() => useRetryJob(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(baseInput)
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mockedRetry).toHaveBeenCalledWith(baseInput)
    expect(toastSuccess).toHaveBeenCalledWith("Retried", { duration: 2_000 })
  })

  it("toasts a generic error when retryJob fails (non-locked)", async () => {
    mockedRetry.mockResolvedValue({
      status: "error",
      error: { kind: "invalidJobState", message: "wrong state", docUrl: null },
    })
    const { result } = renderHook(() => useRetryJob(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(baseInput)
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(toastError).toHaveBeenCalledWith(expect.stringContaining("Retry failed"), undefined)
  })

  it("surfaces the unlock hint verbatim when error is connectionLocked", async () => {
    mockedRetry.mockResolvedValue({
      status: "error",
      error: {
        kind: "connectionLocked",
        message: "Connection prod is locked: Press cmd-shift-U to unlock for this session.",
        docUrl: null,
      },
    })
    const { result } = renderHook(() => useRetryJob(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(baseInput)
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    // Expect the toast message to include the keystroke verbatim,
    // without a generic "Retry failed:" prefix.
    expect(toastError).toHaveBeenCalledWith(expect.stringContaining("cmd-shift-U"), undefined)
    expect(toastError.mock.calls[0]?.[0]).not.toContain("Retry failed")
  })

  it("optimistically flips the cached job state to waiting then rolls back on error", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const cachedJob: Job = {
      jobRef: { queueId: "bull:emails", queueName: "emails", id: "42" },
      name: "send-email",
      data: "{}",
      opts: "{}",
      state: "failed",
      timestamp: 1700000000000,
      attemptsMade: 1,
      failedReason: "kaboom",
      stacktrace: [],
      processedOn: null,
      finishedOn: null,
      returnValue: null,
      parentKey: null,
      progress: null,
      delay: null,
    }
    client.setQueryData(["job", "conn-x", "bull:emails", "42"], cachedJob)
    mockedRetry.mockResolvedValue({
      status: "error",
      error: { kind: "redis", message: "boom", docUrl: null },
    })
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    )
    const { result } = renderHook(() => useRetryJob(), { wrapper })
    await act(async () => {
      result.current.mutate(baseInput)
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    // After rollback, the cached job's state is back to "failed".
    const cached = client.getQueryData<Job>(["job", "conn-x", "bull:emails", "42"])
    expect(cached?.state).toBe("failed")
  })
})

// ─── useRemoveJob ────────────────────────────────────────────────────────

describe("useRemoveJob", () => {
  it("fires removeJob and toasts on success", async () => {
    mockedRemove.mockResolvedValue({
      status: "ok",
      data: { jobId: "42", newState: null, action: "remove" },
    })
    const { result } = renderHook(() => useRemoveJob(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(baseInput)
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mockedRemove).toHaveBeenCalledWith(baseInput)
    expect(toastSuccess).toHaveBeenCalledWith("Removed", { duration: 2_000 })
  })

  it("toasts an error when removeJob fails", async () => {
    mockedRemove.mockResolvedValue({
      status: "error",
      error: { kind: "jobNotFound", message: "not found", docUrl: null },
    })
    const { result } = renderHook(() => useRemoveJob(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(baseInput)
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(toastError).toHaveBeenCalledWith(expect.stringContaining("Remove failed"), undefined)
  })
})

// ─── usePromoteJob ───────────────────────────────────────────────────────

describe("usePromoteJob", () => {
  it("fires promoteJob and toasts on success", async () => {
    mockedPromote.mockResolvedValue({
      status: "ok",
      data: { jobId: "42", newState: "waiting", action: "promote" },
    })
    const { result } = renderHook(() => usePromoteJob(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(baseInput)
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mockedPromote).toHaveBeenCalledWith(baseInput)
    expect(toastSuccess).toHaveBeenCalledWith("Promoted", { duration: 2_000 })
  })

  it("toasts an error when promoteJob fails", async () => {
    mockedPromote.mockResolvedValue({
      status: "error",
      error: { kind: "invalidJobState", message: "wrong state", docUrl: null },
    })
    const { result } = renderHook(() => usePromoteJob(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate(baseInput)
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(toastError).toHaveBeenCalledWith(expect.stringContaining("Promote failed"), undefined)
  })
})

// ─── The row leaves the list at once ──────────────────────────

describe("optimistic list update", () => {
  const listKey = (state: string) => ["jobs", "conn-x", "bull:emails", state, 0]
  const row = (id: string) =>
    ({ jobRef: { queueId: baseInput.queueId, queueName: "emails", id } }) as unknown as Job
  const seeded = () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    client.setQueryData(listKey("failed"), { jobs: [row("41"), row("42")], total: 2 })
    client.setQueryData(listKey("waiting"), { jobs: [row("7")], total: 1 })
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    )
    return { client, wrapper }
  }
  const ids = (client: QueryClient, state: string) =>
    (client.getQueryData(listKey(state)) as { jobs: Job[] }).jobs.map((j) => j.jobRef.id)

  it("a retried job leaves Failed before Redis answers", async () => {
    let answer: (v: unknown) => void = () => {}
    mockedRetry.mockReturnValue(new Promise((r) => (answer = r)))
    const { client, wrapper } = seeded()
    const { result } = renderHook(() => useRetryJob(), { wrapper })
    act(() => result.current.mutate(baseInput))
    await waitFor(() => expect(ids(client, "failed")).toEqual(["41"]))
    expect((client.getQueryData(listKey("failed")) as { total: number }).total).toBe(1)
    expect(ids(client, "waiting")).toEqual(["7"])
    answer({ status: "ok", data: { jobId: "42", newState: "waiting", action: "retry" } })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
  })

  it("comes back when the action fails", async () => {
    mockedRemove.mockResolvedValue({
      status: "error",
      error: { kind: "connectionLocked", message: "locked", docUrl: null },
    })
    const { client, wrapper } = seeded()
    const { result } = renderHook(() => useRemoveJob(), { wrapper })
    await act(async () => {
      result.current.mutate(baseInput)
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(ids(client, "failed")).toEqual(["41", "42"])
  })
})
