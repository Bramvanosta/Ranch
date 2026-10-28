import { waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

// Mock the bindings + useQueues before importing the hook under test.
vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listJobs: vi.fn() },
}))
vi.mock("@/data/queues/queries", () => ({
  useQueues: vi.fn(),
}))

import { useQueues } from "@/data/queues/queries"
import { commands } from "@/lib/tauri/bindings"
import { aJob, aQueue } from "@/test/factories"
import { renderHookWithProviders } from "@/test/renderWithProviders"
import { err, ok } from "@/test/results"

import { useCrossQueueJobs } from "./queries"

const listJobs = vi.mocked(commands.listJobs)

/** The hook only reads `data`, so the rest of the query result is left out. */
function withQueues(...names: string[]) {
  vi.mocked(useQueues).mockReturnValue({
    data: names.map((name) => aQueue(name)),
  } as unknown as ReturnType<typeof useQueues>)
}

afterEach(() => {
  vi.resetAllMocks()
})

describe("useCrossQueueJobs", () => {
  it("aggregates jobs + totals across queues on the happy path", async () => {
    withQueues("emails", "images", "reports")
    listJobs.mockImplementation(async (args) => {
      const name = args.queueId.slice("bull:".length) as "emails" | "images" | "reports"
      const total = { emails: 50, images: 22, reports: 5 }[name] ?? 0
      const jobs = [aJob({ name: `${name}-job`, id: "1", timestamp: total, queue: name })]
      return ok({ jobs, total, state: "waiting", hasMore: false, scanned: null, scanCapped: false })
    })
    const { result } = renderHookWithProviders(() =>
      useCrossQueueJobs({ connectionId: "c1", state: "waiting" }),
    )
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data?.jobs).toHaveLength(3)
    expect(result.current.data?.aggregateTotal).toBe(50 + 22 + 5)
    expect(result.current.data?.perQueueErrors).toEqual([])
    // Sorted newest-first by timestamp.
    expect(result.current.data?.jobs.map((j) => j.timestamp)).toEqual([50, 22, 5])
  })

  it("tolerates per-queue failures, still returns successful queues", async () => {
    withQueues("emails", "broken")
    listJobs.mockImplementation(async (args) => {
      if (args.queueId === "bull:broken") {
        return err("redis", "Connection lost")
      }
      return ok({
        jobs: [aJob({ name: "a", id: "1", timestamp: 100, queue: "emails" })],
        total: 10,
        state: "waiting",
        hasMore: false,
        scanned: null,
        scanCapped: false,
      })
    })
    const { result } = renderHookWithProviders(() =>
      useCrossQueueJobs({ connectionId: "c1", state: "waiting" }),
    )
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data?.jobs).toHaveLength(1)
    expect(result.current.data?.aggregateTotal).toBe(10)
    expect(result.current.data?.perQueueErrors).toEqual([
      { queueName: "broken", message: "Connection lost" },
    ])
  })

  it("returns an empty result when the connection has no queues", async () => {
    withQueues()
    const { result } = renderHookWithProviders(() =>
      useCrossQueueJobs({ connectionId: "c1", state: "waiting" }),
    )
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data).toEqual({
      jobs: [],
      aggregateTotal: 0,
      perQueueErrors: [],
    })
    expect(listJobs).not.toHaveBeenCalled()
  })
})
