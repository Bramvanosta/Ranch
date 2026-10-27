import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

// Mock the bindings + useQueues before importing the hook under test.
vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listJobs: vi.fn() },
}))
vi.mock("@/features/queues/useQueues", () => ({
  useQueues: vi.fn(),
}))

import { useQueues } from "@/features/queues/useQueues"
import { commands, type Job, type Queue } from "@/lib/tauri/bindings"

import { useCrossQueueJobs } from "./useCrossQueueJobs"

const mockedListJobs = commands.listJobs as unknown as ReturnType<typeof vi.fn>
const mockedUseQueues = useQueues as unknown as ReturnType<typeof vi.fn>

afterEach(() => {
  vi.resetAllMocks()
})

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

function makeQueue(name: string): Queue {
  return {
    queueRef: { prefix: "bull", name },
    counters: {
      waiting: 0,
      active: 0,
      completed: 0,
      failed: 0,
      delayed: 0,
      prioritized: 0,
      paused: 0,
    },
    isPaused: false,
  }
}

function makeJob(name: string, id: string, timestamp: number, queueName: string): Job {
  return {
    jobRef: { queue: { prefix: "bull", name: queueName }, id },
    name,
    state: "waiting",
    data: "{}",
    opts: "{}",
    attemptsMade: 0,
    timestamp,
    processedOn: null,
    finishedOn: null,
    delay: null,
    failedReason: null,
    stacktrace: [],
    returnValue: null,
    parentKey: null,
    progress: null,
  }
}

describe("useCrossQueueJobs", () => {
  it("aggregates jobs + totals across queues on the happy path", async () => {
    mockedUseQueues.mockReturnValue({
      data: [makeQueue("emails"), makeQueue("images"), makeQueue("reports")],
    })
    mockedListJobs.mockImplementation((args: { queueRef: { name: string } }) => {
      const name = args.queueRef.name
      const total = { emails: 50, images: 22, reports: 5 }[name] ?? 0
      const jobs = [makeJob(`${name}-job`, "1", total, name)]
      return Promise.resolve({
        status: "ok",
        data: { jobs, total, state: "waiting", hasMore: false },
      })
    })
    const { result } = renderHook(
      () => useCrossQueueJobs({ connectionId: "c1", state: "waiting" }),
      { wrapper: wrap() },
    )
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data?.jobs).toHaveLength(3)
    expect(result.current.data?.aggregateTotal).toBe(50 + 22 + 5)
    expect(result.current.data?.perQueueErrors).toEqual([])
    // Sorted newest-first by timestamp.
    expect(result.current.data?.jobs.map((j) => j.timestamp)).toEqual([50, 22, 5])
  })

  it("tolerates per-queue failures — still returns successful queues", async () => {
    mockedUseQueues.mockReturnValue({
      data: [makeQueue("emails"), makeQueue("broken")],
    })
    mockedListJobs.mockImplementation((args: { queueRef: { name: string } }) => {
      if (args.queueRef.name === "broken") {
        return Promise.resolve({
          status: "error",
          error: { kind: "redis", message: "Connection lost", docUrl: null },
        })
      }
      return Promise.resolve({
        status: "ok",
        data: {
          jobs: [makeJob("a", "1", 100, "emails")],
          total: 10,
          state: "waiting",
          hasMore: false,
        },
      })
    })
    const { result } = renderHook(
      () => useCrossQueueJobs({ connectionId: "c1", state: "waiting" }),
      { wrapper: wrap() },
    )
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data?.jobs).toHaveLength(1)
    expect(result.current.data?.aggregateTotal).toBe(10)
    expect(result.current.data?.perQueueErrors).toEqual([
      { queueName: "broken", message: "Connection lost" },
    ])
  })

  it("returns an empty result when the connection has no queues", async () => {
    mockedUseQueues.mockReturnValue({ data: [] })
    const { result } = renderHook(
      () => useCrossQueueJobs({ connectionId: "c1", state: "waiting" }),
      { wrapper: wrap() },
    )
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data).toEqual({
      jobs: [],
      aggregateTotal: 0,
      perQueueErrors: [],
    })
    expect(mockedListJobs).not.toHaveBeenCalled()
  })
})
