import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listCronJobs: vi.fn() },
}))

import { commands } from "@/lib/tauri/bindings"

import { useCronJobs } from "./useCronJobs"

const mocked = commands as unknown as { listCronJobs: ReturnType<typeof vi.fn> }

afterEach(() => {
  vi.resetAllMocks()
})

function wrapper(retry: boolean | ((n: number, e: Error) => boolean) = false) {
  const client = new QueryClient({ defaultOptions: { queries: { retry } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

const queueRef = { prefix: "bull", name: "emails" }

describe("useCronJobs", () => {
  it("returns the list of schedulers on success", async () => {
    mocked.listCronJobs.mockResolvedValue({
      status: "ok",
      data: [
        {
          key: "daily-report",
          name: "send-daily-report",
          pattern: "0 9 * * *",
          every: null,
          tz: "Europe/Paris",
          offset: 0,
          iterationCount: 1,
          nextRun: 1_778_569_200_000,
          endDate: null,
          template: '{"reportType":"summary"}',
        },
      ],
    })
    const { result } = renderHook(() => useCronJobs({ connectionId: "c1", queueRef }), {
      wrapper: wrapper(),
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data).toHaveLength(1)
    expect(result.current.data?.[0].key).toBe("daily-report")
  })

  it("does NOT retry on schemaNotRecognized", async () => {
    mocked.listCronJobs.mockResolvedValue({
      status: "error",
      error: { kind: "schemaNotRecognized", message: "BullMQ 7.x", docUrl: null },
    })
    const { result } = renderHook(() => useCronJobs({ connectionId: "c1", queueRef }), {
      wrapper: wrapper(undefined as unknown as boolean),
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(mocked.listCronJobs).toHaveBeenCalledTimes(1)
  })

  it("passes through an empty list cleanly", async () => {
    mocked.listCronJobs.mockResolvedValue({ status: "ok", data: [] })
    const { result } = renderHook(() => useCronJobs({ connectionId: "c1", queueRef }), {
      wrapper: wrapper(),
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data).toEqual([])
  })
})
