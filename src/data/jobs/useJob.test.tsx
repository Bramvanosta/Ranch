import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { getJob: vi.fn() },
}))

import { commands } from "@/lib/tauri/bindings"

import { useJob } from "./queries"

const mocked = commands as unknown as { getJob: ReturnType<typeof vi.fn> }

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

describe("useJob", () => {
  it("returns the decoded job on success", async () => {
    mocked.getJob.mockResolvedValue({
      status: "ok",
      data: {
        jobRef: { queue: queueId, id: "1" },
        name: "send-email",
        state: "failed",
        data: "{}",
        opts: "{}",
        attemptsMade: 3,
        timestamp: 0,
        processedOn: 100,
        finishedOn: 200,
        delay: null,
        failedReason: "Connection refused",
        stacktrace: ["Error: x"],
        returnValue: null,
        parentKey: null,
        progress: null,
      },
    })
    const { result } = renderHook(() => useJob({ connectionId: "c1", queueId, jobId: "1" }), {
      wrapper: wrapper(),
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data?.failedReason).toBe("Connection refused")
  })

  it("does NOT retry on jobNotFound, surfaces immediately", async () => {
    mocked.getJob.mockResolvedValue({
      status: "error",
      error: { kind: "jobNotFound", message: "Job not found: emails/999", docUrl: null },
    })
    const { result } = renderHook(
      () => useJob({ connectionId: "c1", queueId, jobId: "999" }),
      // Use the hook's own retry policy.
      { wrapper: wrapper(undefined as unknown as boolean) },
    )
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(mocked.getJob).toHaveBeenCalledTimes(1)
  })

  it("does NOT retry on schemaNotRecognized", async () => {
    mocked.getJob.mockResolvedValue({
      status: "error",
      error: { kind: "schemaNotRecognized", message: "BullMQ 7.x", docUrl: null },
    })
    const { result } = renderHook(() => useJob({ connectionId: "c1", queueId, jobId: "1" }), {
      wrapper: wrapper(undefined as unknown as boolean),
    })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(mocked.getJob).toHaveBeenCalledTimes(1)
  })
})
