import { renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/data/queues/queries", () => ({ useQueues: vi.fn() }))

import { useQueues } from "@/data/queues/queries"
import { aQueue } from "@/test/factories"

import { useRouteQueue } from "./useRouteQueue"

function withQueues(queues: ReturnType<typeof aQueue>[] | undefined) {
  vi.mocked(useQueues).mockReturnValue({ data: queues } as unknown as ReturnType<typeof useQueues>)
}

afterEach(() => vi.resetAllMocks())

describe("useRouteQueue", () => {
  it("finds the queue the address names by its id", () => {
    withQueues([aQueue("emails"), aQueue("images")])
    const { result } = renderHook(() => useRouteQueue("c1", "bull:images"))
    expect(result.current.queue?.name).toBe("images")
    expect(result.current).toMatchObject({
      queueId: "bull:images",
      queueName: "images",
      moveTo: null,
    })
  })

  it("moves an address that names the queue to its id", () => {
    withQueues([aQueue("emails", { id: "app:jobs:emails" })])
    const { result } = renderHook(() => useRouteQueue("c1", "emails"))
    expect(result.current.queueName).toBe("emails")
    expect(result.current.moveTo).toBe("app:jobs:emails")
  })

  it("keeps the address as it is while the queues are not listed yet", () => {
    withQueues(undefined)
    const { result } = renderHook(() => useRouteQueue("c1", "bull:emails"))
    expect(result.current).toMatchObject({
      queue: undefined,
      queueId: "bull:emails",
      moveTo: null,
    })
  })
})
