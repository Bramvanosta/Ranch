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
import type { JobSelectionEntry } from "@/stores/useJobSelectionStore"

import {
  useBulkPromoteCrossQueue,
  useBulkRemoveCrossQueue,
  useBulkRetryCrossQueue,
} from "./useBulkActionCrossQueue"

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

function entry(queue: string, id: string): JobSelectionEntry {
  return {
    jobRef: { queue: { prefix: "bull", name: queue }, id },
    state: "failed",
  }
}

function groupsOf(spec: Record<string, string[]>): Map<string, JobSelectionEntry[]> {
  const map = new Map<string, JobSelectionEntry[]>()
  for (const [queue, ids] of Object.entries(spec)) {
    map.set(
      queue,
      ids.map((id) => entry(queue, id)),
    )
  }
  return map
}

describe("useBulkRetryCrossQueue", () => {
  it("fans out one bulkRetryJobs call per queue and aggregates successes", async () => {
    mockedRetry.mockImplementation(async ({ queueRef, jobIds }) => ({
      status: "ok",
      data: {
        action: "retry",
        total: jobIds.length,
        succeeded: jobIds.length,
        failed: [],
        cancelled: false,
        _queue: queueRef.name,
      },
    }))
    const { result } = renderHook(() => useBulkRetryCrossQueue(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate({
        connectionId: "conn-x",
        queueGroups: groupsOf({ emails: ["1", "2", "3"], reports: ["10", "20"] }),
      })
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    expect(mockedRetry).toHaveBeenCalledTimes(2)
    const res = result.current.data
    if (!res) throw new Error("expected data")
    expect(res.total).toBe(5)
    expect(res.succeeded).toBe(5)
    expect(res.failed).toHaveLength(0)
    expect(res.perQueue).toHaveLength(2)
    const emails = res.perQueue.find((p) => p.queueName === "emails")
    expect(emails?.succeeded).toBe(3)
    const reports = res.perQueue.find((p) => p.queueName === "reports")
    expect(reports?.succeeded).toBe(2)
  })

  it("collects per-job failures across queues into the aggregate failed array", async () => {
    mockedRetry.mockImplementation(async ({ queueRef, jobIds }) => {
      if (queueRef.name === "emails") {
        return {
          status: "ok",
          data: {
            action: "retry",
            total: jobIds.length,
            succeeded: jobIds.length - 1,
            failed: [{ jobId: jobIds[jobIds.length - 1], reason: "not in failed state" }],
            cancelled: false,
          },
        }
      }
      return {
        status: "ok",
        data: {
          action: "retry",
          total: jobIds.length,
          succeeded: jobIds.length,
          failed: [],
          cancelled: false,
        },
      }
    })
    const { result } = renderHook(() => useBulkRetryCrossQueue(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate({
        connectionId: "conn-x",
        queueGroups: groupsOf({ emails: ["1", "2", "3"], reports: ["10"] }),
      })
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    const res = result.current.data
    if (!res) throw new Error("expected data")
    expect(res.total).toBe(4)
    expect(res.succeeded).toBe(3)
    expect(res.failed).toHaveLength(1)
    expect(res.perQueue.find((p) => p.queueName === "emails")?.failed).toHaveLength(1)
    expect(res.perQueue.find((p) => p.queueName === "reports")?.failed).toHaveLength(0)
  })

  it("captures per-queue Tauri-call rejections into PerQueueOutcome.error (locked one queue)", async () => {
    mockedRetry.mockImplementation(async ({ queueRef, jobIds }) => {
      if (queueRef.name === "emails") {
        return {
          status: "error",
          error: {
            kind: "connectionLocked",
            message: "Connection prod is locked: Press cmd-shift-U to unlock for this session.",
            docUrl: null,
          },
        }
      }
      return {
        status: "ok",
        data: {
          action: "retry",
          total: jobIds.length,
          succeeded: jobIds.length,
          failed: [],
          cancelled: false,
        },
      }
    })
    const { result } = renderHook(() => useBulkRetryCrossQueue(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate({
        connectionId: "conn-x",
        queueGroups: groupsOf({ emails: ["1", "2"], reports: ["10"] }),
      })
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    const res = result.current.data
    if (!res) throw new Error("expected data")
    expect(res.total).toBe(3)
    expect(res.succeeded).toBe(1) // only reports succeeded
    const emails = res.perQueue.find((p) => p.queueName === "emails")
    expect(emails?.error).toContain("cmd-shift-U")
    expect(emails?.succeeded).toBe(0)
    expect(emails?.total).toBe(2)
  })

  it("when all queues fail with connectionLocked, every PerQueueOutcome carries the lock message", async () => {
    mockedRetry.mockResolvedValue({
      status: "error",
      error: {
        kind: "connectionLocked",
        message: "Connection prod is locked: Press cmd-shift-U to unlock for this session.",
        docUrl: null,
      },
    })
    const { result } = renderHook(() => useBulkRetryCrossQueue(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate({
        connectionId: "conn-x",
        queueGroups: groupsOf({ emails: ["1"], reports: ["10"], images: ["100"] }),
      })
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    const res = result.current.data
    if (!res) throw new Error("expected data")
    expect(res.perQueue.every((p) => p.error?.includes("cmd-shift-U"))).toBe(true)
    expect(res.succeeded).toBe(0)
    expect(res.total).toBe(3)
  })
})

describe("useBulkRemoveCrossQueue", () => {
  it("aggregates remove across queues", async () => {
    mockedRemove.mockImplementation(async ({ jobIds }) => ({
      status: "ok",
      data: {
        action: "remove",
        total: jobIds.length,
        succeeded: jobIds.length,
        failed: [],
        cancelled: false,
      },
    }))
    const { result } = renderHook(() => useBulkRemoveCrossQueue(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate({
        connectionId: "conn-x",
        queueGroups: groupsOf({ emails: ["1"], reports: ["10"] }),
      })
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mockedRemove).toHaveBeenCalledTimes(2)
    expect(result.current.data?.action).toBe("remove")
    expect(result.current.data?.succeeded).toBe(2)
  })
})

describe("useBulkPromoteCrossQueue", () => {
  it("aggregates promote across queues", async () => {
    mockedPromote.mockImplementation(async ({ jobIds }) => ({
      status: "ok",
      data: {
        action: "promote",
        total: jobIds.length,
        succeeded: jobIds.length,
        failed: [],
        cancelled: false,
      },
    }))
    const { result } = renderHook(() => useBulkPromoteCrossQueue(), { wrapper: wrap() })
    await act(async () => {
      result.current.mutate({
        connectionId: "conn-x",
        queueGroups: groupsOf({ emails: ["1", "2"], reports: ["10"] }),
      })
    })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mockedPromote).toHaveBeenCalledTimes(2)
    expect(result.current.data?.action).toBe("promote")
    expect(result.current.data?.succeeded).toBe(3)
  })
})
