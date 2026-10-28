import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listJobs: vi.fn() },
}))

import { isPageSize, DEFAULT_PAGE_SIZE as PAGE_SIZE } from "@/domain/pageSize"
import { commands } from "@/lib/tauri/bindings"
import { useJobs } from "./queries"

const mocked = commands as unknown as { listJobs: ReturnType<typeof vi.fn> }

afterEach(() => {
  vi.resetAllMocks()
})

function wrapper(retry: boolean | ((n: number, e: Error) => boolean) = false) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry } },
  })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

const queueId = "bull:emails"

describe("useJobs", () => {
  it("calls list_jobs with the right offset for a given page", async () => {
    mocked.listJobs.mockResolvedValue({
      status: "ok",
      data: { jobs: [], total: 0, state: "failed", hasMore: false },
    })
    const { result } = renderHook(
      () =>
        useJobs({
          connectionId: "conn-x",
          queueId,
          state: "failed",
          page: 2,
        }),
      { wrapper: wrapper() },
    )
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mocked.listJobs).toHaveBeenCalledWith(
      expect.objectContaining({
        connectionId: "conn-x",
        queueId,
        state: "failed",
        offset: 2 * PAGE_SIZE,
        limit: PAGE_SIZE,
        newestFirst: true,
      }),
    )
  })

  it("does NOT retry on schemaNotRecognized, fails immediately", async () => {
    mocked.listJobs.mockResolvedValue({
      status: "error",
      error: {
        kind: "schemaNotRecognized",
        message: "BullMQ schema not recognized",
        docUrl: null,
      },
    })
    const { result } = renderHook(
      () =>
        useJobs({
          connectionId: "conn-bad",
          queueId,
          state: "failed",
          page: 0,
        }),
      // Use the hook's own retry policy (don't override).
      { wrapper: wrapper(undefined as unknown as boolean) },
    )
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(mocked.listJobs).toHaveBeenCalledTimes(1)
  })

  it("returns the unwrapped data on success", async () => {
    mocked.listJobs.mockResolvedValue({
      status: "ok",
      data: {
        jobs: [
          {
            jobRef: { queue: queueId, id: "1" },
            name: "n",
            state: "waiting",
            data: "{}",
            opts: "{}",
            attemptsMade: 0,
            timestamp: 0,
            processedOn: null,
            finishedOn: null,
            delay: null,
            failedReason: null,
            stacktrace: [],
            returnValue: null,
            parentKey: null,
            progress: null,
          },
        ],
        total: 1,
        state: "waiting",
        hasMore: false,
      },
    })
    const { result } = renderHook(
      () =>
        useJobs({
          connectionId: "conn-x",
          queueId,
          state: "waiting",
          page: 0,
        }),
      { wrapper: wrapper() },
    )
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data?.total).toBe(1)
    expect(result.current.data?.jobs[0].jobRef.id).toBe("1")
  })
  // ── Server-side search, reason, sort and page size ───────────────────
  it("sends search, reason, sort and page size to the server", async () => {
    mocked.listJobs.mockResolvedValue({
      status: "ok",
      data: { jobs: [], total: 0, state: "failed", hasMore: false, scanned: 0, scanCapped: false },
    })
    const { result } = renderHook(
      () =>
        useJobs({
          connectionId: "conn-x",
          queueId,
          state: "failed",
          page: 3,
          pageSize: 100,
          search: "  invoice ",
          reason: "",
          sort: { field: "name", descending: true },
        }),
      { wrapper: wrapper() },
    )
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mocked.listJobs).toHaveBeenCalledWith({
      connectionId: "conn-x",
      queueId,
      state: "failed",
      offset: 300,
      limit: 100,
      newestFirst: true,
      search: "invoice",
      reason: null,
      sort: { field: "name", descending: true },
    })
  })

  it("does not fetch when disabled", async () => {
    renderHook(
      () => useJobs({ connectionId: "c", queueId, state: "failed", page: 0, enabled: false }),
      { wrapper: wrapper() },
    )
    await new Promise((r) => setTimeout(r, 20))
    expect(mocked.listJobs).not.toHaveBeenCalled()
  })

  it("only accepts the offered page sizes", () => {
    expect([25, 50, 100, 250].every(isPageSize)).toBe(true)
    expect(isPageSize(10)).toBe(false)
    expect(isPageSize(Number.NaN)).toBe(false)
  })

  it("does NOT retry a superseded scan: it would cancel the newer one", async () => {
    mocked.listJobs.mockResolvedValue({
      status: "error",
      error: { kind: "superseded", message: "Superseded by a newer job query", docUrl: null },
    })
    const { result } = renderHook(
      () => useJobs({ connectionId: "c", queueId, state: "failed", page: 0, search: "ne" }),
      // No client-level override: the hook's own retry policy decides.
      { wrapper: wrapper((n: number) => n < 2) },
    )
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(mocked.listJobs).toHaveBeenCalledTimes(1)
  })
})

describe("useJobs placeholder", () => {
  const page = (queue: string) => ({
    status: "ok",
    data: {
      jobs: [{ jobRef: { queueId: `bull:${queue}`, queueName: queue, id: "1" } }],
      total: 1,
      state: "failed",
      hasMore: false,
    },
  })

  it("keeps the previous rows for another page of the same queue", async () => {
    mocked.listJobs.mockResolvedValueOnce(page("emails")).mockReturnValue(new Promise(() => {}))
    const { result, rerender } = renderHook(
      ({ p }: { p: number }) =>
        useJobs({ connectionId: "conn-x", queueId, state: "failed", page: p }),
      { wrapper: wrapper(), initialProps: { p: 0 } },
    )
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    rerender({ p: 1 })
    expect(result.current.isPlaceholderData).toBe(true)
    expect(result.current.data?.jobs).toHaveLength(1)
  })

  it("never shows another queue's rows under this queue", async () => {
    mocked.listJobs.mockResolvedValueOnce(page("emails")).mockReturnValue(new Promise(() => {}))
    const { result, rerender } = renderHook(
      ({ name }: { name: string }) =>
        useJobs({
          connectionId: "conn-x",
          queueId: `bull:${name}`,
          state: "failed",
          page: 0,
        }),
      { wrapper: wrapper(), initialProps: { name: "emails" } },
    )
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    rerender({ name: "exports" })
    expect(result.current.isPlaceholderData).toBe(false)
    expect(result.current.data).toBeUndefined()
    expect(result.current.isLoading).toBe(true)
  })
})
