import { waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { listCronJobs: vi.fn(), scheduleRunsWithin: vi.fn() },
}))
vi.mock("@/data/queues/queries", () => ({ useQueues: vi.fn() }))
vi.mock("@/data/settings/useRefreshSettings", () => ({ useRefetchInterval: () => false }))
vi.mock("@/lib/polling/sharedPoll", () => ({ useSharedPolls: vi.fn() }))

import { useQueues } from "@/data/queues/queries"
import { useCronJobs } from "@/data/schedulers/queries"
import { commands } from "@/lib/tauri/bindings"
import { aCronJob, aQueue, aQueueId } from "@/test/factories"
import { renderHookWithProviders } from "@/test/renderWithProviders"
import { ok } from "@/test/results"

import { useConnectionSchedulers } from "./useConnectionSchedulers"

afterEach(() => {
  vi.resetAllMocks()
})

describe("useConnectionSchedulers", () => {
  it("shares the queue tab's cached list without changing its shape", async () => {
    vi.mocked(useQueues).mockReturnValue({
      data: [aQueue("emails")],
    } as unknown as ReturnType<typeof useQueues>)
    vi.mocked(commands.listCronJobs).mockResolvedValue(ok([aCronJob()]))
    vi.mocked(commands.scheduleRunsWithin).mockResolvedValue(ok({ nextRunsMs: [1, 2] }))

    // Both readers mounted together, as on a connection's Schedulers view
    // with a queue's tab open: one query key, so one shape.
    const { result } = renderHookWithProviders(() => ({
      connection: useConnectionSchedulers("c1"),
      queue: useCronJobs({ connectionId: "c1", queueId: aQueueId("emails") }),
    }))

    await waitFor(() => expect(result.current.connection.rows).toHaveLength(1))
    expect(result.current.connection.rows[0].queueName).toBe("emails")
    expect(result.current.connection.rows[0].scheduler.key).toBe("nightly")
    await waitFor(() =>
      expect(result.current.queue.data).toEqual([aCronJob({ nextRun: expect.any(Number) })]),
    )
  })
})
